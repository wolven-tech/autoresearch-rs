//! Exact-commit evaluation, frozen selection, and durable Git finalization.

use crate::baseline::{
    BaselineExecutor, RunnerError, StoredRun, ensure_real_directory, redact_failure,
};
use autoresearch_core::{
    CandidateCommit, CandidateDecision, CandidateFinalization, CandidateRecovery,
    CandidateWorkspace, Disposition, EvaluationSnapshot, JournalEvent, RecoveryAction,
    RepositoryInspector, select_candidate,
};
use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec, build_snapshot};
use autoresearch_git::{GitRepository, LockedGitRepository, RunLockGuard};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

/// Fully evaluated and applied candidate, with exact comparable evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateOutcome {
    /// Candidate number from durable preparation event.
    pub index: u32,
    /// Exact candidate commit evaluated.
    pub commit: String,
    /// All declared gates and metrics plus measured complexity.
    pub snapshot: EvaluationSnapshot,
    /// Frozen lexicographic decision and reason.
    pub decision: CandidateDecision,
    /// Confirmed retained-ref effect.
    pub finalization: CandidateFinalization,
}

/// Evaluates one already committed, journal-prepared candidate. Rechecks Git
/// identity before and after every evaluator. Appends decision and evidence
/// before changing retained ref; then reconciles exact keep/discard effect and
/// records finalization. If side effect or final append fails, journal remains
/// at an explicit recovery boundary. No market evidence enters selection.
///
/// # Errors
///
/// Refuses incomplete baseline, mismatched candidate commit, changed worktree,
/// missing/invalid evaluator output, or untrustworthy finalization. On error,
/// no fabricated pass or objective is journaled.
pub fn evaluate_committed_candidate(
    repository: &Path,
    run_id: &str,
    committed: &CandidateCommit,
    executor: &impl BaselineExecutor,
    declared_environment: BTreeMap<String, String>,
) -> Result<CandidateOutcome, RunnerError> {
    let mut stored = StoredRun::load(repository, run_id)?;
    let (index, worktree_id) = match stored.view.recovery_action() {
        RecoveryAction::EvaluateCandidate { index, worktree_id } => (*index, worktree_id.clone()),
        _ => {
            return Err(RunnerError::InvalidState(
                "run is not awaiting candidate evaluation",
            ));
        }
    };
    let baseline = current_best_snapshot(&stored)?;
    let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
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
    if candidate.worktree_id() != worktree_id {
        return Err(RunnerError::InvalidState(
            "candidate worktree differs from journal",
        ));
    }
    git.validate_candidate_commit(&run, &candidate, committed, stored.manifest.scope())?;
    let artifacts = stored.run_directory.join("artifacts");
    ensure_real_directory(&artifacts)?;
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: run_id.into(),
        baseline_commit: stored.base_commit.clone(),
        evaluated_commit: committed.commit_id().to_string(),
        candidate_worktree: candidate.path().to_path_buf(),
        changed_paths: committed
            .changed_paths()
            .iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        declared_environment,
        artifact_directory: artifacts,
        cancellation_id: format!("{run_id}-candidate-{index}"),
    })?;
    let started = Instant::now();
    let mut outputs = Vec::with_capacity(stored.manifest.evaluators().len());
    for evaluator in stored.manifest.evaluators() {
        let result = executor.evaluate(&context, evaluator);
        git.validate_candidate_commit(&run, &candidate, committed, stored.manifest.scope())?;
        match result {
            Ok(output) => outputs.push(output),
            Err(failure) => {
                return Err(RunnerError::CandidateEvaluator {
                    evaluator_id: evaluator.id().into(),
                    failure: redact_failure(&failure),
                });
            }
        }
    }
    let runtime_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let complexity = git.candidate_complexity(
        &run,
        &candidate,
        committed,
        stored.manifest.scope(),
        runtime_ms,
    )?;
    let evaluated = build_snapshot(&context, &stored.manifest, outputs, complexity)?;
    let decision = select_candidate(
        &baseline,
        &evaluated,
        stored.manifest.experiment().objective().name(),
    )?;
    stored.append(JournalEvent::CandidateDecisionRecorded {
        index,
        candidate_commit: committed.commit_id().to_string(),
        snapshot: evaluated.clone(),
        decision: decision.clone(),
    })?;
    let Some(CandidateRecovery::Finalized { outcome, .. }) = git.recover_candidate(&stored.view)?
    else {
        return Err(RunnerError::InvalidState(
            "recorded decision did not finalize",
        ));
    };
    stored.append(JournalEvent::CandidateFinalized {
        index,
        outcome: outcome.clone(),
    })?;
    Ok(CandidateOutcome {
        index,
        commit: committed.commit_id().to_string(),
        snapshot: evaluated,
        decision,
        finalization: outcome,
    })
}

fn current_best_snapshot(stored: &StoredRun) -> Result<EvaluationSnapshot, RunnerError> {
    let mut best = stored
        .view
        .baseline()
        .cloned()
        .ok_or(RunnerError::InvalidState("baseline snapshot is missing"))?;
    let mut pending = None;
    for entry in &stored.entries {
        match &entry.event {
            JournalEvent::CandidateDecisionRecorded {
                snapshot, decision, ..
            } => {
                let expected = select_candidate(
                    &best,
                    snapshot,
                    stored.manifest.experiment().objective().name(),
                )?;
                if &expected != decision {
                    return Err(RunnerError::InvalidState(
                        "prior decision differs from frozen policy",
                    ));
                }
                pending = Some((snapshot.clone(), decision.disposition));
            }
            JournalEvent::CandidateFinalized { outcome, .. } => {
                let Some((snapshot, disposition)) = pending.take() else {
                    return Err(RunnerError::InvalidState("finalization lacks decision"));
                };
                if disposition == Disposition::Keep {
                    if !matches!(outcome, CandidateFinalization::Kept { .. }) {
                        return Err(RunnerError::InvalidState("kept snapshot was not retained"));
                    }
                    best = snapshot;
                }
            }
            _ => {}
        }
    }
    if pending.is_some() {
        return Err(RunnerError::InvalidState(
            "prior candidate has pending finalization",
        ));
    }
    Ok(best)
}
