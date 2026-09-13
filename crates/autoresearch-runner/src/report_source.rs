//! Verified frozen source bundle for deterministic read-only report builders.

use crate::baseline::{RunnerError, StoredRun};
use crate::environment::EnvironmentRecord;
use autoresearch_config::{FrozenIdentity, ValidatedManifest};
use autoresearch_core::JournalEntry;
use std::path::{Path, PathBuf};

/// Fully revalidated run inputs, exposed without mutation capability.
#[derive(Debug, Clone)]
pub struct ReportSource {
    /// Frozen run identifier.
    pub run_id: String,
    /// Canonical caller repository.
    pub repository: PathBuf,
    /// Run-owned evidence directory.
    pub run_directory: PathBuf,
    /// Original base commit.
    pub base_commit: String,
    /// Replayed retained current-best commit.
    pub current_commit: String,
    /// Frozen validated manifest.
    pub manifest: ValidatedManifest,
    /// Complete gap-free replayed journal.
    pub entries: Vec<JournalEntry>,
    /// Frozen input digest components.
    pub identity: FrozenIdentity,
    /// Captured redaction-safe host and declared command facts, when recorded.
    pub environment: Option<EnvironmentRecord>,
    /// Journal-bound digest of environment record, when recorded.
    pub environment_fingerprint: Option<String>,
}

/// Loads report source only after journal replay, frozen-content rehash, and
/// exact-base Git comparison succeed. Does not run evaluators or mutate state.
///
/// # Errors
///
/// Refuses missing, dirty, tampered, or malformed run evidence.
pub fn load_report_source(repository: &Path, run_id: &str) -> Result<ReportSource, RunnerError> {
    let stored = StoredRun::load(repository, run_id)?;
    Ok(ReportSource {
        run_id: run_id.into(),
        repository: stored.root,
        run_directory: stored.run_directory,
        base_commit: stored.base_commit,
        current_commit: stored.view.current_commit().into(),
        manifest: stored.manifest,
        entries: stored.entries,
        identity: stored.identity,
        environment: stored.environment,
        environment_fingerprint: stored.view.environment_fingerprint().map(str::to_owned),
    })
}
