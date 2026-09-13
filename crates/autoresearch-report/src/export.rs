//! Explicit, bounded, redacted evidence export outside target repository.

use crate::{BoardError, RunReportV1, render_board, to_json_bytes};
use autoresearch_core::RepoPath;
use autoresearch_market::ReceiptSource;
use autoresearch_runner::ReportSource;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

const MAX_SELECTED_ARTIFACTS: usize = 16;
const MAX_ARTIFACT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;

/// Export result with no remote publication side effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportResult {
    /// Newly created, complete bundle directory.
    pub directory: PathBuf,
    /// Number of report and selected artifact files, excluding manifest.
    pub file_count: usize,
    /// SHA-256 of sanitized report JSON.
    pub report_sha256: String,
    /// Relative provenance manifest path.
    pub manifest_path: String,
}

/// Export cannot safely create complete approved bundle.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Output-root or file operation failed.
    #[error("bundle I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Report serialization failed.
    #[error("bundle JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Board refused an artifact reference.
    #[error(transparent)]
    Board(#[from] BoardError),
    /// Output root would mutate target repository or is not a real directory.
    #[error("export root must be an existing real directory outside target repository")]
    UnsafeRoot,
    /// Complete export with same run ID already exists.
    #[error("export destination already exists; existing bundle left untouched")]
    ExistingDestination,
    /// Selected artifact count or path is unsafe.
    #[error("artifact selection exceeds bound or contains unsafe path")]
    UnsafeSelection,
    /// Selected path is not referenced in validated run report.
    #[error("selected artifact is not declared by report")]
    UndeclaredArtifact,
    /// Selected file is absent, symlinked, or not regular.
    #[error("selected artifact is absent, symlinked, or not a regular file")]
    UnsafeArtifact,
    /// Selected file exceeds bounded allowance.
    #[error("selected artifact exceeds byte limit")]
    ArtifactTooLarge,
    /// Selected file contains known token or API-key pattern.
    #[error("selected artifact contains credential pattern; artifact not exported")]
    SensitiveArtifact,
    /// Process-log-looking artifacts cannot be evidence-bundle members.
    #[error("raw process log artifacts cannot be exported")]
    RawProcessLog,
    /// Structural provenance or path would be altered by redaction.
    #[error("structural report field contains credential pattern; export refused")]
    SensitiveIdentifier,
    /// Report schema cannot be rendered or rehydrated.
    #[error("report schema is unsupported for export")]
    InvalidReport,
}

/// One verifiable member in complete bundle.
#[derive(Debug, Serialize)]
struct BundleFile {
    path: String,
    sha256: String,
    bytes: usize,
}

/// Manifest deliberately has no self-hash or unstable wall-clock field.
#[derive(Debug, Serialize)]
struct ProvenanceManifest {
    schema_version: u8,
    run_id: String,
    base_commit: String,
    current_best_commit: String,
    frozen_contract_sha256: String,
    environment_fingerprint_sha256: Option<String>,
    redaction_policy_version: u8,
    selected_artifacts: Vec<String>,
    files: Vec<BundleFile>,
}

/// Builds a new complete local bundle. `export_root` must already exist
/// outside target repository. Only report-selected artifacts are copied;
/// journal, program, manifest source, environment variables, mutable
/// credentials, and raw process logs are never copied.
///
/// # Errors
///
/// Refuses unsafe root, existing destination, undeclared or symlinked
/// artifacts, credential patterns, invalid report, or filesystem failure.
pub fn export_bundle(
    source: &ReportSource,
    report: &RunReportV1,
    export_root: &Path,
    selected_artifacts: &[PathBuf],
) -> Result<ExportResult, ExportError> {
    if report.schema_version != 1
        || report.run_id != source.run_id
        || report.base_commit != source.base_commit
        || report.current_best_commit != source.current_commit
        || report.frozen_identity_sha256 != source.identity.aggregate_sha256
        || report.environment.fingerprint_sha256 != source.environment_fingerprint
    {
        return Err(ExportError::InvalidReport);
    }
    let root = checked_export_root(source, export_root)?;
    let selected = checked_selection(report, selected_artifacts)?;
    let artifact_bytes = selected
        .iter()
        .map(|path| {
            read_selected_artifact(&source.run_directory, path).map(|bytes| (path.clone(), bytes))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sanitized = sanitize_report(report, &selected)?;
    let report_json = to_json_bytes(&sanitized).map_err(|_| ExportError::InvalidReport)?;
    if report_json.len() > MAX_REPORT_BYTES {
        return Err(ExportError::InvalidReport);
    }
    if contains_credential(&String::from_utf8_lossy(&report_json)) {
        return Err(ExportError::SensitiveIdentifier);
    }
    let report_html = render_board(&sanitized, &source.run_directory)?;
    if report_html.len() > MAX_REPORT_BYTES {
        return Err(ExportError::InvalidReport);
    }
    if contains_credential(&String::from_utf8_lossy(&report_html)) {
        return Err(ExportError::SensitiveIdentifier);
    }
    let final_dir = root.join(format!("autoresearch-{}", source.run_id));
    ensure_destination_absent(&final_dir)?;
    let staging = create_staging(&root, &source.run_id)?;
    let mut files = Vec::new();
    for (path, bytes) in &artifact_bytes {
        files.push(write_member(&staging, &format!("artifacts/{path}"), bytes)?);
    }
    files.push(write_member(&staging, "report.json", &report_json)?);
    files.push(write_member(&staging, "report.html", &report_html)?);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = ProvenanceManifest {
        schema_version: 1,
        run_id: source.run_id.clone(),
        base_commit: report.base_commit.clone(),
        current_best_commit: report.current_best_commit.clone(),
        frozen_contract_sha256: report.frozen_identity_sha256.clone(),
        environment_fingerprint_sha256: report.environment.fingerprint_sha256.clone(),
        redaction_policy_version: 1,
        selected_artifacts: selected.into_iter().collect(),
        files,
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    write_member(&staging, "provenance.json", &manifest_bytes)?;
    ensure_destination_absent(&final_dir)?;
    fs::rename(&staging, &final_dir)?;
    Ok(ExportResult {
        directory: final_dir,
        file_count: manifest.files.len(),
        report_sha256: sha256(&report_json),
        manifest_path: "provenance.json".into(),
    })
}

fn ensure_destination_absent(path: &Path) -> Result<(), ExportError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(ExportError::ExistingDestination),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ExportError::Io(error)),
    }
}

fn checked_export_root(source: &ReportSource, path: &Path) -> Result<PathBuf, ExportError> {
    if !path.is_absolute() {
        return Err(ExportError::UnsafeRoot);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| ExportError::UnsafeRoot)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ExportError::UnsafeRoot);
    }
    let root = fs::canonicalize(path)?;
    let repository = fs::canonicalize(&source.repository).map_err(|_| ExportError::UnsafeRoot)?;
    if root.starts_with(&repository) {
        return Err(ExportError::UnsafeRoot);
    }
    Ok(root)
}

fn checked_selection(
    report: &RunReportV1,
    selected: &[PathBuf],
) -> Result<BTreeSet<String>, ExportError> {
    if selected.len() > MAX_SELECTED_ARTIFACTS {
        return Err(ExportError::UnsafeSelection);
    }
    let declared = report
        .candidates
        .iter()
        .flat_map(|candidate| {
            candidate
                .artifacts
                .iter()
                .map(|artifact| (artifact.relative_path.as_str(), artifact.name.as_str()))
        })
        .collect::<Vec<_>>();
    let mut paths = BTreeSet::new();
    for path in selected {
        let name = path.to_str().ok_or(ExportError::UnsafeSelection)?;
        RepoPath::new(name).map_err(|_| ExportError::UnsafeSelection)?;
        let Some((_, artifact_name)) = declared.iter().find(|(path, _)| *path == name) else {
            return Err(ExportError::UndeclaredArtifact);
        };
        if looks_like_process_log(name, artifact_name) {
            return Err(ExportError::RawProcessLog);
        }
        if !paths.insert(name.to_owned()) {
            return Err(ExportError::UnsafeSelection);
        }
    }
    Ok(paths)
}

fn looks_like_process_log(path: &str, name: &str) -> bool {
    let filename = Path::new(path)
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let label = name.to_ascii_lowercase();
    matches!(
        Path::new(&filename).extension().and_then(|part| part.to_str()),
        Some("log" | "jsonl")
    )
        || matches!(filename.as_str(), "log.txt" | "stdout.txt" | "stderr.txt")
        || matches!(
            label.as_str(),
            "log" | "raw log" | "process log" | "stdout" | "stderr"
        )
}

fn read_selected_artifact(run_directory: &Path, path: &str) -> Result<Vec<u8>, ExportError> {
    let root = run_directory.join("artifacts");
    let root_meta = fs::symlink_metadata(&root).map_err(|_| ExportError::UnsafeArtifact)?;
    if !root_meta.is_dir() || root_meta.file_type().is_symlink() {
        return Err(ExportError::UnsafeArtifact);
    }
    let mut current = root;
    for component in Path::new(path).components() {
        let Component::Normal(name) = component else {
            return Err(ExportError::UnsafeArtifact);
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current).map_err(|_| ExportError::UnsafeArtifact)?;
        if metadata.file_type().is_symlink() {
            return Err(ExportError::UnsafeArtifact);
        }
    }
    let metadata = fs::metadata(&current).map_err(|_| ExportError::UnsafeArtifact)?;
    if !metadata.is_file() {
        return Err(ExportError::UnsafeArtifact);
    }
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(ExportError::ArtifactTooLarge);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    fs::File::open(&current)?
        .take(MAX_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(ExportError::ArtifactTooLarge);
    }
    if contains_credential(&String::from_utf8_lossy(&bytes)) {
        return Err(ExportError::SensitiveArtifact);
    }
    Ok(bytes)
}

fn sanitize_report(
    report: &RunReportV1,
    selected: &BTreeSet<String>,
) -> Result<RunReportV1, ExportError> {
    let mut copy = report.clone();
    copy.environment.record = None;
    copy.environment.status = if copy.environment.fingerprint_sha256.is_some() {
        "fingerprint_only_in_export"
    } else {
        "unavailable_in_legacy_run"
    }
    .into();
    for candidate in &mut copy.candidates {
        candidate
            .artifacts
            .retain(|artifact| selected.contains(&artifact.relative_path));
    }
    redact_public_report(&copy)
}

/// Applies same text-boundary redaction used by export to ordinary reports.
pub(crate) fn redact_public_report(report: &RunReportV1) -> Result<RunReportV1, ExportError> {
    assert_structural_fields_safe(report)?;
    let mut value = serde_json::to_value(report)?;
    redact_value(&mut value, None);
    let sanitized: RunReportV1 = serde_json::from_value(value)?;
    if sanitized.schema_version != 1 || sanitized.run_id != report.run_id {
        return Err(ExportError::InvalidReport);
    }
    Ok(sanitized)
}

fn assert_structural_fields_safe(report: &RunReportV1) -> Result<(), ExportError> {
    let fields = [
        report.run_id.as_str(),
        report.base_commit.as_str(),
        report.current_best_commit.as_str(),
        report.frozen_identity_sha256.as_str(),
        report.objective.name.as_str(),
    ];
    if fields.into_iter().any(contains_credential)
        || report
            .environment
            .fingerprint_sha256
            .as_deref()
            .is_some_and(contains_credential)
    {
        return Err(ExportError::SensitiveIdentifier);
    }
    for candidate in &report.candidates {
        if contains_credential(&candidate.parent_commit)
            || candidate
                .candidate_commit
                .as_deref()
                .is_some_and(contains_credential)
            || candidate
                .changed_paths
                .iter()
                .any(|path| contains_credential(path))
            || candidate
                .artifacts
                .iter()
                .any(|artifact| contains_credential(&artifact.relative_path))
            || candidate
                .hard_gates
                .iter()
                .any(|gate| contains_credential(&gate.name))
        {
            return Err(ExportError::SensitiveIdentifier);
        }
    }
    for receipt in &report.market_evidence.receipts {
        let source = match &receipt.declaration.source {
            ReceiptSource::File { path } => path.to_string_lossy(),
            ReceiptSource::Uri { uri } => uri.as_str().into(),
        };
        if contains_credential(&receipt.receipt_path.to_string_lossy())
            || contains_credential(&source)
        {
            return Err(ExportError::SensitiveIdentifier);
        }
    }
    Ok(())
}

fn redact_value(value: &mut Value, key: Option<&str>) {
    match value {
        Value::String(text) => {
            if key.is_some_and(sensitive_key) {
                *text = "[REDACTED]".into();
            } else {
                *text = redact_text(text);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| redact_value(item, None)),
        Value::Object(items) => {
            for (key, item) in items {
                redact_value(item, Some(key));
            }
        }
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key" | "secret" | "secret_key" | "token" | "access_token" | "authorization"
    )
}

fn credential_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)(?:\bBearer\s+[A-Za-z0-9._~+/=-]{8,}|\b(?:api[_-]?key|access[_-]?token|secret|authorization)\s*[:=]\s*['"]?[A-Za-z0-9._~+/=-]{8,}|\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{8,}|\bgh[pousr]_[A-Za-z0-9_]{16,}|\bAKIA[0-9A-Z]{16}|\bxox[baprs]-[A-Za-z0-9-]{12,})"#)
            .expect("static credential pattern")
    })
}

fn contains_credential(text: &str) -> bool {
    credential_pattern().is_match(text)
}

fn redact_text(text: &str) -> String {
    credential_pattern()
        .replace_all(text, "[REDACTED]")
        .into_owned()
}

fn create_staging(root: &Path, run_id: &str) -> Result<PathBuf, ExportError> {
    for suffix in 0..32 {
        let path = root.join(format!(
            ".autoresearch-{run_id}-{}-{suffix}.pending",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(ExportError::Io(error)),
        }
    }
    Err(ExportError::ExistingDestination)
}

fn write_member(root: &Path, relative: &str, bytes: &[u8]) -> Result<BundleFile, ExportError> {
    RepoPath::new(relative).map_err(|_| ExportError::UnsafeSelection)?;
    let mut path = root.to_path_buf();
    let components = Path::new(relative).components().collect::<Vec<_>>();
    for component in &components[..components.len() - 1] {
        let Component::Normal(name) = component else {
            return Err(ExportError::UnsafeSelection);
        };
        path.push(name);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(ExportError::UnsafeRoot);
                }
            }
            Err(error) => return Err(ExportError::Io(error)),
        }
    }
    let Component::Normal(name) = components[components.len() - 1] else {
        return Err(ExportError::UnsafeSelection);
    };
    path.push(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(BundleFile {
        path: relative.into(),
        sha256: sha256(bytes),
        bytes: bytes.len(),
    })
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}
