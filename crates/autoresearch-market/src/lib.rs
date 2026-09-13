//! Read-only commercial receipt imports. This crate has no evaluator, Git,
//! provider API, or network client dependency; receipts cannot affect code
//! selection and do not themselves prove a product bet gate.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use url::Url;

const MAX_RECEIPTS: usize = 256;
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

/// Supported evidence categories; outreach and use do not imply purchase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptType {
    /// Payment-provider receipt.
    Payment,
    /// Fulfilment or delivery receipt.
    Fulfilment,
    /// Refund-provider receipt.
    Refund,
    /// Qualified-use record with human provenance.
    QualifiedUse,
    /// Outreach attempt or response record, not a conversion.
    Outreach,
}

/// Provenance pointer. HTTPS is not fetched; file bytes are hashed locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptSource {
    /// Relative path under declared evidence root.
    File {
        /// Source file path.
        path: PathBuf,
    },
    /// Credential-free durable HTTPS URL.
    Uri {
        /// Source URI.
        uri: String,
    },
}

/// Whether importer could independently verify source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestStatus {
    /// Local source file digest matched declaration.
    VerifiedLocalFile,
    /// Remote URI was not fetched; digest remains declaration only.
    DeclaredRemoteUri,
}

/// One strictly parsed receipt sidecar supplied by operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptDeclaration {
    /// Protocol version, currently 1.
    pub schema_version: u8,
    /// Commercial receipt category.
    pub receipt_type: ReceiptType,
    /// Source URI or local relative file path.
    pub source: ReceiptSource,
    /// Event timestamp, milliseconds since Unix epoch.
    pub occurred_at_unix_ms: u64,
    /// Lowercase SHA-256 digest of source evidence bytes.
    pub evidence_sha256: String,
}

/// Imported receipt with sidecar digest and explicit trust level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketReceipt {
    /// Relative sidecar path under evidence root.
    pub receipt_path: PathBuf,
    /// Exact sidecar bytes digest.
    pub receipt_sha256: String,
    /// Validated operator declaration.
    pub declaration: ReceiptDeclaration,
    /// Whether source bytes were checked, not whether customer claim is true.
    pub digest_status: DigestStatus,
}

/// Read-only aggregate, deliberately separate from evaluator snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketEvidenceLedger {
    /// Imported provenance-bound receipts in requested path order.
    pub receipts: Vec<MarketReceipt>,
}

/// Invalid or unverifiable provenance. No fallback score is produced.
#[derive(Debug, Error)]
pub enum ImportError {
    /// Receipt count exceeds bounded importer ceiling.
    #[error("too many receipt declarations")]
    TooManyReceipts,
    /// Sidecar or source path is unsafe.
    #[error("unsafe evidence path: {}", path.display())]
    UnsafePath {
        /// Rejected path.
        path: PathBuf,
    },
    /// Duplicate sidecar path would double count one input.
    #[error("duplicate receipt sidecar: {}", path.display())]
    DuplicateReceipt {
        /// Duplicate path.
        path: PathBuf,
    },
    /// File read or metadata query failed.
    #[error("could not read evidence file {}: {source}", path.display())]
    Io {
        /// Affected file path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// File is larger than bounded importer allowance.
    #[error("evidence file exceeds byte limit: {}", path.display())]
    FileTooLarge {
        /// Affected file path.
        path: PathBuf,
    },
    /// Sidecar JSON did not match strict receipt schema.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Missing or malformed timestamp or digest.
    #[error("receipt provenance is missing or invalid: {0}")]
    InvalidProvenance(&'static str),
    /// Local source digest does not match sidecar declaration.
    #[error("local evidence digest mismatch for {}", path.display())]
    DigestMismatch {
        /// Source file path.
        path: PathBuf,
    },
}

/// Imports operator-selected sidecars without mutating them or calling any
/// provider API. File sources must sit under evidence root and match SHA-256;
/// HTTPS sources are not fetched and remain marked unverified.
///
/// # Errors
///
/// Rejects unsafe paths, missing provenance, non-HTTPS or credential-bearing
/// URI, unknown category, digest mismatch, oversized input, or duplicates.
pub fn import_receipts(
    evidence_root: &Path,
    receipt_paths: &[PathBuf],
) -> Result<MarketEvidenceLedger, ImportError> {
    if receipt_paths.len() > MAX_RECEIPTS {
        return Err(ImportError::TooManyReceipts);
    }
    let root = fs::canonicalize(evidence_root).map_err(|source| ImportError::Io {
        path: evidence_root.to_path_buf(),
        source,
    })?;
    if !root.is_dir() {
        return Err(ImportError::UnsafePath { path: root });
    }
    let mut seen = BTreeSet::new();
    let mut receipts = Vec::with_capacity(receipt_paths.len());
    for relative in receipt_paths {
        if !seen.insert(relative.clone()) {
            return Err(ImportError::DuplicateReceipt {
                path: relative.clone(),
            });
        }
        let path = checked_file(&root, relative, MAX_RECEIPT_BYTES)?;
        let bytes = read_bounded(&path, MAX_RECEIPT_BYTES)?;
        let declaration: ReceiptDeclaration = serde_json::from_slice(&bytes)?;
        validate_declaration(&declaration)?;
        let digest_status = match &declaration.source {
            ReceiptSource::File { path: source } => {
                let source_path = checked_file(&root, source, MAX_SOURCE_BYTES)?;
                let source_bytes = read_bounded(&source_path, MAX_SOURCE_BYTES)?;
                if sha256(&source_bytes) != declaration.evidence_sha256 {
                    return Err(ImportError::DigestMismatch { path: source_path });
                }
                DigestStatus::VerifiedLocalFile
            }
            ReceiptSource::Uri { .. } => DigestStatus::DeclaredRemoteUri,
        };
        receipts.push(MarketReceipt {
            receipt_path: relative.clone(),
            receipt_sha256: sha256(&bytes),
            declaration,
            digest_status,
        });
    }
    Ok(MarketEvidenceLedger { receipts })
}

fn validate_declaration(declaration: &ReceiptDeclaration) -> Result<(), ImportError> {
    if declaration.schema_version != 1 {
        return Err(ImportError::InvalidProvenance("unknown schema version"));
    }
    if declaration.occurred_at_unix_ms == 0 {
        return Err(ImportError::InvalidProvenance("timestamp must be nonzero"));
    }
    if declaration.evidence_sha256.len() != 64
        || !declaration
            .evidence_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ImportError::InvalidProvenance(
            "evidence digest must be lowercase SHA-256",
        ));
    }
    match &declaration.source {
        ReceiptSource::File { path } => {
            checked_relative(path)?;
        }
        ReceiptSource::Uri { uri } => {
            let url = Url::parse(uri)
                .map_err(|_| ImportError::InvalidProvenance("source URI is invalid"))?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(ImportError::InvalidProvenance(
                    "source URI must be credential-free HTTPS without query or fragment",
                ));
            }
        }
    }
    Ok(())
}

fn checked_relative(path: &Path) -> Result<(), ImportError> {
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ImportError::UnsafePath {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn checked_file(root: &Path, relative: &Path, limit: u64) -> Result<PathBuf, ImportError> {
    checked_relative(relative)?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&path).map_err(|source| ImportError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ImportError::UnsafePath { path });
        }
    }
    let metadata = fs::metadata(&path).map_err(|source| ImportError::Io {
        path: path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ImportError::UnsafePath { path });
    }
    if metadata.len() > limit {
        return Err(ImportError::FileTooLarge { path });
    }
    Ok(path)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ImportError> {
    let file = File::open(path).map_err(|source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ImportError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len())
        .ok()
        .is_none_or(|len| len > limit)
    {
        return Err(ImportError::FileTooLarge {
            path: path.to_path_buf(),
        });
    }
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}
