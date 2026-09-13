//! Journal-directed recovery without re-deciding durable outcomes.

use crate::baseline::{
    BaselineCapture, BaselineExecutor, RunnerError, StoredRun, capture_existing_baseline,
};
use crate::candidate::{CandidateOutcome, evaluate_committed_candidate};
use autoresearch_core::{
    CandidateFinalization, CandidateRecovery, JournalEvent, RecoveredCandidateState,
    RecoveryAction, RepositoryInspector,
};
use autoresearch_git::{GitRepository, LockedGitRepository, RunLockGuard};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One safe recovery step derived from journal and exact Git observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// Incomplete baseline was captured or failed explicitly.
    Baseline(BaselineCapture),
    /// Prepared worktree requires operator/agent mutation; no changes guessed.
    NeedsMutation {
        /// Active candidate number.
        index: u32,
        /// Existing isolated candidate worktree.
        worktree: PathBuf,
    },
    /// Committed candidate finished only missing evaluator work.
    Candidate(CandidateOutcome),
    /// Durable decision applied exactly; evaluators were not rerun.
    Finalized {
        /// Candidate number.
        index: u32,
        /// Confirmed side effect.
        outcome: CandidateFinalization,
    },
    /// No active candidate; next serial scheduler decision may start.
    ReadyForCandidate {
        /// Next one-based candidate number.
        index: u32,
    },
    /// Run already stopped.
    Finished,
}

/// Advances exactly one journal recovery action. Incomplete evaluator outputs
/// are replayed and revalidated by candidate runner; completed outputs are not
/// rerun. Recorded decisions go straight to Git finalization and never face
/// newly sampled scores. Ambiguous Git state returns error and preserves
/// evidence for operator inspection.
///
/// # Errors
///
/// Refuses frozen identity changes, diverged refs, foreign worktrees, and any
/// recovery transition whose Git effect cannot be proven exactly.
pub fn resume_run(
    repository: &Path,
    run_id: &str,
    executor: &impl BaselineExecutor,
    declared_environment: BTreeMap<String, String>,
) -> Result<ResumeOutcome, RunnerError> {
    let mut stored = StoredRun::load(repository, run_id)?;
    match stored.view.recovery_action().clone() {
        RecoveryAction::CaptureBaseline => {
            capture_existing_baseline(repository, run_id, executor, declared_environment)
                .map(ResumeOutcome::Baseline)
        }
        RecoveryAction::PrepareCandidate { index, .. } => {
            Ok(ResumeOutcome::ReadyForCandidate { index })
        }
        RecoveryAction::EvaluateCandidate { index, .. } => {
            let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
            let lock = RunLockGuard::acquire(&snapshot, run_id)?;
            let git = LockedGitRepository::new(&snapshot, &lock)?;
            let Some(CandidateRecovery::EvaluationReady {
                run,
                candidate,
                state,
            }) = git.recover_candidate(&stored.view)?
            else {
                return Err(RunnerError::InvalidState(
                    "prepared candidate did not recover",
                ));
            };
            match state {
                RecoveredCandidateState::Prepared => Ok(ResumeOutcome::NeedsMutation {
                    index,
                    worktree: candidate.path().to_path_buf(),
                }),
                RecoveredCandidateState::Committed { .. } => {
                    let committed =
                        git.recover_committed_candidate(&run, &candidate, stored.manifest.scope())?;
                    drop(git);
                    drop(lock);
                    evaluate_committed_candidate(
                        repository,
                        run_id,
                        &committed,
                        executor,
                        declared_environment,
                    )
                    .map(ResumeOutcome::Candidate)
                }
            }
        }
        RecoveryAction::FinalizeCandidate { index, .. } => {
            let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
            let lock = RunLockGuard::acquire(&snapshot, run_id)?;
            let git = LockedGitRepository::new(&snapshot, &lock)?;
            let Some(CandidateRecovery::Finalized { outcome, .. }) =
                git.recover_candidate(&stored.view)?
            else {
                return Err(RunnerError::InvalidState(
                    "recorded decision did not finalize",
                ));
            };
            stored.append(JournalEvent::CandidateFinalized {
                index,
                outcome: outcome.clone(),
            })?;
            Ok(ResumeOutcome::Finalized { index, outcome })
        }
        RecoveryAction::Finished => Ok(ResumeOutcome::Finished),
        RecoveryAction::StartRun => Err(RunnerError::InvalidState("run journal is empty")),
    }
}
