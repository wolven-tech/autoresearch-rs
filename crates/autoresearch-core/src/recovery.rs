//! Pure journal-to-workspace recovery contracts.

use crate::{
    CandidateDecision, CandidateFinalization, CandidateWorkspace, CommitId, RecoveryAction,
    RepositoryValueError, RunId, RunView, RunWorkspace, WorkspaceValueError,
};
use std::error::Error;
use std::path::Path;
use thiserror::Error;

/// Journal recovery state cannot form one canonical candidate operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecoveryValueError {
    /// Journal commit text is not a full Git object identity.
    #[error(transparent)]
    InvalidCommit(#[from] RepositoryValueError),
    /// Journal run or candidate identity is not safe for Git and paths.
    #[error(transparent)]
    InvalidWorkspace(#[from] WorkspaceValueError),
    /// Durable worktree label differs from deterministic candidate identity.
    #[error("journal worktree ID mismatch: expected `{expected}`, got `{actual}`")]
    WorktreeIdMismatch {
        /// Identity derived from run and candidate number.
        expected: String,
        /// Identity stored in journal.
        actual: String,
    },
}

/// Candidate Git operation derived only from replayed journal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateRecoveryRequest {
    /// Ensure candidate workspace exists, then continue mutation or evaluation.
    Evaluate {
        /// Retained run state declared by journal.
        run: RunWorkspace,
        /// Deterministic candidate workspace declared by journal.
        candidate: CandidateWorkspace,
    },
    /// Apply recorded keep/discard decision and clean candidate workspace.
    Finalize {
        /// Retained run state before recorded decision is applied.
        run: RunWorkspace,
        /// Deterministic candidate workspace declared by journal.
        candidate: CandidateWorkspace,
        /// Exact candidate commit evaluated by frozen policy.
        candidate_commit: CommitId,
        /// Exact durable decision; infrastructure must not recompute it.
        decision: CandidateDecision,
    },
}

impl CandidateRecoveryRequest {
    /// Builds candidate recovery request from one valid replay projection.
    ///
    /// Returns `None` when next transition does not require candidate Git I/O.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryValueError`] when journal identifiers cannot form
    /// canonical run/candidate values or recorded worktree identity disagrees.
    pub fn from_run_view(
        view: &RunView,
        worktree_root: &Path,
    ) -> Result<Option<Self>, RecoveryValueError> {
        match view.recovery_action() {
            RecoveryAction::EvaluateCandidate { index, worktree_id } => {
                let run = run_workspace(view)?;
                let candidate = checked_candidate(&run, *index, worktree_id, worktree_root)?;
                Ok(Some(Self::Evaluate { run, candidate }))
            }
            RecoveryAction::FinalizeCandidate {
                index,
                worktree_id,
                candidate_commit,
                decision,
            } => {
                let run = run_workspace(view)?;
                let candidate = checked_candidate(&run, *index, worktree_id, worktree_root)?;
                Ok(Some(Self::Finalize {
                    run,
                    candidate,
                    candidate_commit: CommitId::new(candidate_commit.clone())?,
                    decision: decision.clone(),
                }))
            }
            RecoveryAction::StartRun
            | RecoveryAction::CaptureBaseline
            | RecoveryAction::PrepareCandidate { .. }
            | RecoveryAction::Finished => Ok(None),
        }
    }
}

fn run_workspace(view: &RunView) -> Result<RunWorkspace, RecoveryValueError> {
    Ok(RunWorkspace::new(
        RunId::new(view.run_id())?,
        CommitId::new(view.base_commit())?,
        CommitId::new(view.current_commit())?,
    ))
}

/// Observable candidate state after recovery reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveredCandidateState {
    /// Workspace is clean at parent commit; mutation has not produced a commit.
    Prepared,
    /// Workspace is clean at one direct-child commit; evaluators may resume.
    Committed {
        /// Candidate revision ready for evaluation.
        commit: CommitId,
    },
}

/// Result of reconciling one journal-driven candidate Git operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateRecovery {
    /// Candidate exists with unambiguous mutation/evaluation state.
    EvaluationReady {
        /// Retained run state still matching journal.
        run: RunWorkspace,
        /// Proven run-owned candidate workspace.
        candidate: CandidateWorkspace,
        /// Whether candidate still needs mutation or only evaluation.
        state: RecoveredCandidateState,
    },
    /// Recorded keep/discard effects and candidate cleanup have completed.
    Finalized {
        /// Retained run state after decision application.
        run: RunWorkspace,
        /// Journal event payload caller may append idempotently.
        outcome: CandidateFinalization,
    },
}

/// Port required to reconcile journal state with isolated candidate storage.
pub trait CandidateRecoveryManager {
    /// Adapter-specific diagnostic error.
    type Error: Error + Send + Sync + 'static;

    /// Reconciles candidate Git state required by replay projection.
    ///
    /// Returns `None` when replay requests no candidate Git operation.
    ///
    /// # Errors
    ///
    /// Returns adapter diagnostics when durable journal and observable storage
    /// cannot be reconciled without guessing.
    fn recover_candidate(&self, view: &RunView) -> Result<Option<CandidateRecovery>, Self::Error>;
}

fn checked_candidate(
    run: &RunWorkspace,
    index: u32,
    worktree_id: &str,
    worktree_root: &Path,
) -> Result<CandidateWorkspace, RecoveryValueError> {
    let candidate = CandidateWorkspace::new(run, index, worktree_root)?;
    if candidate.worktree_id() != worktree_id {
        return Err(RecoveryValueError::WorktreeIdMismatch {
            expected: candidate.worktree_id().to_owned(),
            actual: worktree_id.to_owned(),
        });
    }
    Ok(candidate)
}
