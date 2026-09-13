//! Read-only run projection and explicit operator stop at stable boundary.

use crate::baseline::{RunnerError, StoredRun};
use autoresearch_core::{EvaluationSnapshot, JournalEvent, RecoveryAction, RepositoryInspector};
use autoresearch_git::{GitRepository, LockedGitRepository, RunLockGuard};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Journal-derived status; no raw evaluator output or mutation side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunStatus {
    /// Frozen run identifier.
    pub run_id: String,
    /// Durable evidence directory.
    pub run_directory: PathBuf,
    /// Frozen base commit.
    pub base_commit: String,
    /// Journal current-best commit.
    pub current_commit: String,
    /// Number of finalized candidates.
    pub completed_candidates: u32,
    /// Stable next recovery action.
    pub recovery_action: String,
    /// Active one-based candidate number, if any.
    pub active_candidate: Option<u32>,
    /// Frozen baseline evidence, if captured.
    pub baseline_snapshot: Option<EvaluationSnapshot>,
    /// Terminal stop reason, if any.
    pub stop_reason: Option<String>,
}

/// Inspects frozen journal and source identity without writing run, branch,
/// worktree, or caller checkout.
///
/// # Errors
///
/// Rejects missing, dirty, or self-inconsistent run state.
pub fn inspect_run(repository: &Path, run_id: &str) -> Result<RunStatus, RunnerError> {
    let stored = StoredRun::load(repository, run_id)?;
    let (recovery_action, active_candidate) = match stored.view.recovery_action() {
        RecoveryAction::StartRun => ("start_run", None),
        RecoveryAction::CaptureBaseline => ("capture_baseline", None),
        RecoveryAction::PrepareCandidate { index, .. } => ("prepare_candidate", Some(*index)),
        RecoveryAction::EvaluateCandidate { index, .. } => ("evaluate_candidate", Some(*index)),
        RecoveryAction::FinalizeCandidate { index, .. } => ("finalize_candidate", Some(*index)),
        RecoveryAction::Finished => ("finished", None),
    };
    Ok(RunStatus {
        run_id: run_id.into(),
        run_directory: stored.run_directory,
        base_commit: stored.base_commit,
        current_commit: stored.view.current_commit().into(),
        completed_candidates: stored.view.completed_candidates(),
        recovery_action: recovery_action.into(),
        active_candidate,
        baseline_snapshot: stored.view.baseline().cloned(),
        stop_reason: stored.view.stop_reason().map(str::to_owned),
    })
}

/// Records operator cancellation only between candidates. Active candidate
/// cannot be silently discarded; operator must finish or inspect it first.
///
/// # Errors
///
/// Refuses incomplete baseline, active candidate, divergent ref, or dirty
/// caller checkout before appending durable stop.
pub fn stop_run(repository: &Path, run_id: &str) -> Result<RunStatus, RunnerError> {
    let mut stored = StoredRun::load(repository, run_id)?;
    if !matches!(
        stored.view.recovery_action(),
        RecoveryAction::PrepareCandidate { .. }
    ) {
        return Err(RunnerError::InvalidState(
            "run can stop only between candidates",
        ));
    }
    let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
    let lock = RunLockGuard::acquire(&snapshot, run_id)?;
    let git = LockedGitRepository::new(&snapshot, &lock)?;
    let run = git.open_run()?;
    if run.head_commit().as_str() != stored.view.current_commit() {
        return Err(RunnerError::InvalidState(
            "retained run ref differs from journal current commit",
        ));
    }
    stored.append(JournalEvent::RunStopped {
        reason: "operator_cancelled".into(),
    })?;
    inspect_run(&stored.root, run_id)
}
