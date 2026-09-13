//! Filesystem transaction for freezing a new run before evaluation.

use crate::{AppError, BaselineReport, git_output, read_manifest};
use autoresearch_config::FrozenIdentity;
use autoresearch_core::{JournalEntry, JournalEvent};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn capture(repository: &Path) -> Result<BaselineReport, AppError> {
    let root = repository_root(repository)?;
    let manifest_path = root.join("autoresearch.toml");
    let program_path = root.join("program.md");
    let product_gate_path = root.join("docs/BET.md");
    ensure_run_state_ignored(&root)?;
    ensure_frozen_inputs_clean(&root, product_gate_path.is_file())?;

    let manifest_source = read_required(&manifest_path)?;
    let manifest = read_manifest(&manifest_source)?;
    let program = fs::read(&program_path).map_err(|source| AppError::Io {
        operation: "read program",
        path: program_path.clone(),
        source,
    })?;
    if program.iter().all(u8::is_ascii_whitespace) {
        return Err(AppError::BlankProgram);
    }
    let product_gate = if product_gate_path.is_file() {
        Some(fs::read(&product_gate_path).map_err(|source| AppError::Io {
            operation: "read product gate",
            path: product_gate_path.clone(),
            source,
        })?)
    } else {
        None
    };
    let identity = FrozenIdentity::capture(
        &manifest,
        &program,
        &BTreeMap::new(),
        product_gate.as_deref(),
    )?;
    let commit = git_output(&root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let run_id_prefix = new_run_id(&commit)?;
    let (run_id, run_directory) = create_run_directory(&root, &run_id_prefix)?;
    let frozen_directory = run_directory.join("frozen");
    fs::create_dir(&frozen_directory).map_err(|source| AppError::Io {
        operation: "create frozen input directory",
        path: frozen_directory.clone(),
        source,
    })?;

    write_new(
        &frozen_directory.join("autoresearch.toml"),
        manifest_source.as_bytes(),
    )?;
    write_new(&frozen_directory.join("program.md"), &program)?;
    if let Some(gate) = product_gate {
        write_new(&frozen_directory.join("product-gate.md"), &gate)?;
    }

    let mut identity_json = serde_json::to_vec_pretty(&identity)?;
    identity_json.push(b'\n');
    write_new(&run_directory.join("identity.json"), &identity_json)?;

    let journal_entry = JournalEntry {
        sequence: 0,
        run_id: run_id.clone(),
        event: JournalEvent::RunStarted {
            base_commit: commit.clone(),
            frozen_identity: identity.aggregate_sha256.clone(),
        },
    };
    let mut journal_json = serde_json::to_vec(&journal_entry)?;
    journal_json.push(b'\n');
    write_new(&run_directory.join("journal.jsonl"), &journal_json)?;

    Ok(BaselineReport {
        run_id,
        run_directory,
        base_commit: commit,
        frozen_identity: identity.aggregate_sha256,
        next_action: "capture_baseline".into(),
        evidence_status: "pending evaluator evidence".into(),
        snapshot: None,
        failed_evaluator: None,
        failure: None,
    })
}

fn repository_root(repository: &Path) -> Result<PathBuf, AppError> {
    let root = git_output(repository, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(root))
}

fn ensure_frozen_inputs_clean(root: &Path, has_product_gate: bool) -> Result<(), AppError> {
    let mut paths = vec!["autoresearch.toml", "program.md"];
    if has_product_gate {
        paths.push("docs/BET.md");
    }
    for path in &paths {
        if git_output(root, &["ls-files", "--error-unmatch", "--", path]).is_err() {
            return Err(AppError::DirtyFrozenInputs(format!(
                "{path} is not tracked"
            )));
        }
    }
    let mut args = vec!["status", "--porcelain=v1", "--untracked-files=all", "--"];
    args.extend(paths);
    let status = git_output(root, &args)?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(AppError::DirtyFrozenInputs(status))
    }
}

fn ensure_run_state_ignored(root: &Path) -> Result<(), AppError> {
    if git_output(root, &["check-ignore", "-q", ".autoresearch/probe"]).is_ok() {
        Ok(())
    } else {
        Err(AppError::RunStateNotIgnored)
    }
}

fn read_required(path: &Path) -> Result<String, AppError> {
    fs::read_to_string(path).map_err(|source| AppError::Io {
        operation: "read manifest",
        path: path.to_path_buf(),
        source,
    })
}

fn new_run_id(commit: &str) -> Result<String, AppError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::ClockBeforeEpoch)?;
    let short_commit = commit.chars().take(8).collect::<String>();
    Ok(format!(
        "run-{}-{}-{short_commit}",
        elapsed.as_millis(),
        std::process::id()
    ))
}

fn create_run_directory(root: &Path, run_id: &str) -> Result<(String, PathBuf), AppError> {
    let runs = root.join(".autoresearch/runs");
    fs::create_dir_all(&runs).map_err(|source| AppError::Io {
        operation: "create runs directory",
        path: runs.clone(),
        source,
    })?;

    for suffix in 0_u16..1_000 {
        let name = if suffix == 0 {
            run_id.to_owned()
        } else {
            format!("{run_id}-{suffix}")
        };
        let candidate = runs.join(&name);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok((name, candidate)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(AppError::Io {
                    operation: "create run directory",
                    path: candidate,
                    source,
                });
            }
        }
    }
    Err(AppError::RunIdCollision)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| AppError::Io {
            operation: "create frozen artifact",
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(bytes).map_err(|source| AppError::Io {
        operation: "write frozen artifact",
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all().map_err(|source| AppError::Io {
        operation: "sync frozen artifact",
        path: path.to_path_buf(),
        source,
    })
}
