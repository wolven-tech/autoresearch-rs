//! One bounded, journal-directed mutation step for CLI operators.

use crate::baseline::{BaselineExecutor, RunnerError, StoredRun};
use crate::mutation::{MutationRequest, submit_manual_candidate};
use crate::{CommandMutationAdapter, ResumeOutcome, resume_run};
use autoresearch_core::{CandidateWorkspace, JournalEvent, RecoveryAction, RepositoryInspector};
use autoresearch_evaluator::CancellationToken;
use autoresearch_git::{GitError, GitRepository, LockedGitRepository, RunLockGuard};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Mutation execution stays local and serial. Command mode requires a separate
/// exact executable allowlist; manifest alone cannot authorize execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMutationMode {
    /// Operator edits the isolated candidate worktree and submits next call.
    Manual,
    /// Run only this explicitly allowlisted absolute executable.
    Command {
        /// Absolute binary separately authorized by operator.
        executable: PathBuf,
    },
}

/// One durable runner transition exposed by CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStep {
    /// Candidate worktree is ready for operator changes.
    AwaitingMutation {
        /// One-based candidate index.
        index: u32,
        /// Isolated candidate worktree path.
        worktree: PathBuf,
    },
    /// Candidate completed evaluation and exact Git finalization.
    Evaluated(crate::CandidateOutcome),
    /// Previously recorded decision was finalized without reevaluation.
    Finalized {
        /// Candidate number.
        index: u32,
        /// Exact kept/discarded effect.
        outcome: autoresearch_core::CandidateFinalization,
    },
    /// Frozen candidate ceiling or prior stop prevents further work.
    Stopped {
        /// Durable run stop reason.
        reason: String,
    },
}

/// Advances one candidate through preparation, local mutation, or recovery.
/// Each call uses one frozen run ID and at most one serial candidate. A manual
/// call without hypothesis only prepares/returns isolated worktree. Supplying
/// hypothesis submits existing edits through Phase 2 containment, then
/// evaluates. Command mode needs exact binary allowlist and refuses declared
/// external authority because no OS sandbox is installed here.
///
/// # Errors
///
/// Refuses untrusted frozen state, dirty caller checkout, missing command
/// authority, changed candidate, or any incomplete evaluator evidence.
pub fn advance_run_once(
    repository: &Path,
    run_id: &str,
    mode: &RunMutationMode,
    hypothesis: Option<&str>,
    executor: &impl BaselineExecutor,
) -> Result<RunStep, RunnerError> {
    let mut stored = StoredRun::load(repository, run_id)?;
    if stored.manifest.experiment().budget().max_candidates == 0
        || stored.manifest.experiment().budget().max_failures == 0
        || stored.manifest.experiment().budget().wall_clock_seconds == 0
    {
        return Err(RunnerError::InvalidState("run budget is unbounded"));
    }
    if let RunMutationMode::Command { executable } = mode {
        if hypothesis.is_none() {
            return Err(RunnerError::InvalidState(
                "command mutation requires hypothesis",
            ));
        }
        if !stored.manifest.authority().allowed().is_empty() {
            return Err(RunnerError::InvalidState(
                "command mutation cannot grant external authority",
            ));
        }
        let executable_matches = std::fs::canonicalize(executable)
            .ok()
            .zip(std::fs::canonicalize(stored.manifest.agent().program()).ok())
            .is_some_and(|(allowed, declared)| allowed == declared);
        if !executable.is_absolute() || !executable_matches {
            return Err(RunnerError::InvalidState(
                "command executable lacks exact allowlist authority",
            ));
        }
    }

    match stored.view.recovery_action().clone() {
        RecoveryAction::CaptureBaseline => Err(RunnerError::InvalidState(
            "run needs baseline evidence before mutation",
        )),
        RecoveryAction::PrepareCandidate {
            index,
            parent_commit,
        } => prepare_next(
            &mut stored,
            index,
            &parent_commit,
            mode,
            hypothesis,
            executor,
        ),
        RecoveryAction::EvaluateCandidate { index, .. } => {
            advance_prepared(&stored, index, mode, hypothesis, executor)
        }
        RecoveryAction::FinalizeCandidate { .. } => {
            map_resume(resume_run(repository, run_id, executor, BTreeMap::new())?)
        }
        RecoveryAction::Finished => Ok(RunStep::Stopped {
            reason: stored.view.stop_reason().unwrap_or("finished").into(),
        }),
        RecoveryAction::StartRun => Err(RunnerError::InvalidState("run journal is empty")),
    }
}

fn prepare_next(
    stored: &mut StoredRun,
    index: u32,
    parent_commit: &str,
    mode: &RunMutationMode,
    hypothesis: Option<&str>,
    executor: &impl BaselineExecutor,
) -> Result<RunStep, RunnerError> {
    if matches!(mode, RunMutationMode::Manual) && hypothesis.is_some() {
        return Err(RunnerError::InvalidState(
            "manual candidate must be prepared before submission",
        ));
    }
    if index > stored.manifest.experiment().budget().max_candidates {
        stored.append(JournalEvent::RunStopped {
            reason: "candidate_limit".into(),
        })?;
        return Ok(RunStep::Stopped {
            reason: "candidate_limit".into(),
        });
    }
    let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
    let run_id = stored.view.run_id().to_owned();
    let lock = RunLockGuard::acquire(&snapshot, &run_id)?;
    let git = LockedGitRepository::new(&snapshot, &lock)?;
    let run = git.open_run()?;
    if run.head_commit().as_str() != parent_commit {
        return Err(RunnerError::InvalidState(
            "retained run ref differs from journal current commit",
        ));
    }
    let candidate =
        CandidateWorkspace::new(&run, index, &stored.root.join(".autoresearch/worktrees"))
            .map_err(|_| RunnerError::InvalidState("candidate path cannot be derived"))?;
    stored.append(JournalEvent::CandidatePrepared {
        index,
        parent_commit: parent_commit.into(),
        worktree_id: candidate.worktree_id().into(),
    })?;
    git.prepare_candidate(&run, index)?;
    if matches!(mode, RunMutationMode::Manual) {
        return Ok(RunStep::AwaitingMutation {
            index,
            worktree: candidate.path().to_path_buf(),
        });
    }
    mutate_candidate(stored, &git, &run, &candidate, mode, hypothesis)?;
    drop(git);
    drop(lock);
    map_resume(resume_run(
        &stored.root,
        &run_id,
        executor,
        BTreeMap::new(),
    )?)
}

fn advance_prepared(
    stored: &StoredRun,
    index: u32,
    mode: &RunMutationMode,
    hypothesis: Option<&str>,
    executor: &impl BaselineExecutor,
) -> Result<RunStep, RunnerError> {
    let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
    let run_id = stored.view.run_id();
    let lock = RunLockGuard::acquire(&snapshot, run_id)?;
    let git = LockedGitRepository::new(&snapshot, &lock)?;
    let run = git.open_run()?;
    if run.head_commit().as_str() != stored.view.current_commit() {
        return Err(RunnerError::InvalidState(
            "retained run ref differs from journal current commit",
        ));
    }
    let candidate =
        CandidateWorkspace::new(&run, index, &stored.root.join(".autoresearch/worktrees"))
            .map_err(|_| RunnerError::InvalidState("candidate path cannot be derived"))?;
    if hypothesis.is_none() {
        drop(git);
        drop(lock);
        return match resume_run(&stored.root, run_id, executor, BTreeMap::new()) {
            Err(RunnerError::Git(GitError::DirtyRepository { .. })) => {
                Ok(RunStep::AwaitingMutation {
                    index,
                    worktree: candidate.path().to_path_buf(),
                })
            }
            other => map_resume(other?),
        };
    }
    mutate_candidate(stored, &git, &run, &candidate, mode, hypothesis)?;
    drop(git);
    drop(lock);
    map_resume(resume_run(&stored.root, run_id, executor, BTreeMap::new())?)
}

fn mutate_candidate(
    stored: &StoredRun,
    git: &LockedGitRepository<'_>,
    run: &autoresearch_core::RunWorkspace,
    candidate: &CandidateWorkspace,
    mode: &RunMutationMode,
    hypothesis: Option<&str>,
) -> Result<(), RunnerError> {
    let request = MutationRequest::new(
        candidate,
        &stored.manifest,
        &stored.program,
        &stored.identity,
        Vec::new(),
        hypothesis.ok_or(RunnerError::InvalidState(
            "candidate hypothesis is required",
        ))?,
        &format!("{}-candidate-{}", run.run_id(), candidate.index()),
    )?;
    match mode {
        RunMutationMode::Manual => {
            submit_manual_candidate(&stored.root, git, run, candidate, &request)?;
        }
        RunMutationMode::Command { executable } => {
            let adapter = CommandMutationAdapter::new(
                std::slice::from_ref(executable),
                1024 * 1024,
                1024 * 1024,
                CancellationToken::default(),
            )?;
            adapter.execute_and_commit(
                &stored.root,
                git,
                run,
                candidate,
                &request,
                &stored.manifest,
            )?;
        }
    }
    Ok(())
}

fn map_resume(outcome: ResumeOutcome) -> Result<RunStep, RunnerError> {
    match outcome {
        ResumeOutcome::Candidate(outcome) => Ok(RunStep::Evaluated(outcome)),
        ResumeOutcome::NeedsMutation { index, worktree } => {
            Ok(RunStep::AwaitingMutation { index, worktree })
        }
        ResumeOutcome::Finalized { index, outcome } => Ok(RunStep::Finalized { index, outcome }),
        ResumeOutcome::Finished => Ok(RunStep::Stopped {
            reason: "finished".into(),
        }),
        ResumeOutcome::Baseline(_) | ResumeOutcome::ReadyForCandidate { .. } => Err(
            RunnerError::InvalidState("unexpected runner recovery transition"),
        ),
    }
}
