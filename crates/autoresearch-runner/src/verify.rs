//! Independent kept-commit verification without selection or ref mutation.

use crate::baseline::{
    BaselineExecutor, RunnerError, StoredRun, ensure_real_directory, redact_failure,
};
use autoresearch_core::{
    Complexity, Disposition, EvaluationSnapshot, EvaluatorFailure, FailureClass, JournalEvent,
    Measurement, RecoveryAction, RepositoryInspector, RunWorkspace,
};
use autoresearch_evaluator::{EvaluationContext, EvaluationContextSpec, build_snapshot};
use autoresearch_git::{GitRepository, LockedGitRepository, RunLockGuard};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Freshly measured evaluator output. Candidate complexity is intentionally
/// absent: verification does not remeasure selection tie-breakers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FreshEvaluation {
    /// Frozen-declaration ordered measurements from new evaluator execution.
    pub measurements: Vec<Measurement>,
    /// Runtime of this independent evaluator pass.
    pub runtime_ms: u64,
}

/// Separate evidence record; original selection snapshot remains unmodified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationEvidence {
    /// Frozen run identifier.
    pub run_id: String,
    /// Exact previously kept commit.
    pub verified_commit: String,
    /// Existing selection evidence copied for side-by-side inspection.
    pub selection_snapshot: EvaluationSnapshot,
    /// Newly sampled evaluator evidence, never used for selection.
    pub fresh_snapshot: Option<FreshEvaluation>,
    /// Named evaluator that failed fresh run, if any.
    pub failed_evaluator: Option<String>,
    /// Typed, redacted fresh failure, if any.
    pub failure: Option<EvaluatorFailure>,
    /// Matched, drifted, or failed.
    pub status: String,
    /// Durable independent artifact, outside selection journal.
    pub evidence_path: PathBuf,
}

struct FreshResult {
    snapshot: Option<FreshEvaluation>,
    failure: Option<(String, EvaluatorFailure)>,
}

/// Re-runs frozen evaluators at kept commit in isolated detached worktree.
/// Writes one separate verification artifact. Never appends candidate journal,
/// changes retained branch, or promotes market evidence.
///
/// # Errors
///
/// Refuses no kept candidate, pending recovery, divergent branch, unsafe
/// worktree, or invalid frozen identity before claiming fresh verification.
pub fn verify_kept_commit(
    repository: &Path,
    run_id: &str,
    executor: &impl BaselineExecutor,
) -> Result<VerificationEvidence, RunnerError> {
    let stored = StoredRun::load(repository, run_id)?;
    if !matches!(
        stored.view.recovery_action(),
        RecoveryAction::PrepareCandidate { .. } | RecoveryAction::Finished
    ) || stored.view.current_commit() == stored.base_commit
    {
        return Err(RunnerError::InvalidState(
            "no finalized kept candidate is available for verification",
        ));
    }
    let selection_snapshot = selection_for_current(&stored)?;
    let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
    let lock = RunLockGuard::acquire(&snapshot, run_id)?;
    let git = LockedGitRepository::new(&snapshot, &lock)?;
    let run = git.open_run()?;
    if run.head_commit().as_str() != stored.view.current_commit() {
        return Err(RunnerError::InvalidState(
            "retained run ref differs from journal current commit",
        ));
    }
    let (_parent, changed_paths) =
        GitRepository.read_candidate_parent_and_paths(&snapshot, run.head_commit())?;
    for path in &changed_paths {
        stored.manifest.scope().validate(path).map_err(|_| {
            RunnerError::InvalidState("kept commit changed path outside frozen scope")
        })?;
    }
    let worktree = git.prepare_verification(&run)?;
    let verification_dir = allocate_verification_directory(&stored.run_directory)?;
    let artifacts_root = stored.run_directory.join("artifacts");
    ensure_real_directory(&artifacts_root)?;
    let verification_artifacts = artifacts_root.join("verifications");
    ensure_real_directory(&verification_artifacts)?;
    let artifacts = verification_artifacts.join(verification_dir.file_name().ok_or(
        RunnerError::InvalidState("verification directory has no name"),
    )?);
    ensure_real_directory(&artifacts)?;
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: run_id.into(),
        baseline_commit: stored.base_commit.clone(),
        evaluated_commit: run.head_commit().as_str().into(),
        candidate_worktree: worktree,
        changed_paths: changed_paths
            .iter()
            .map(|path| path.as_str().into())
            .collect(),
        declared_environment: BTreeMap::new(),
        artifact_directory: artifacts,
        cancellation_id: format!("{run_id}-verification"),
    })?;
    let FreshResult {
        snapshot: fresh_snapshot,
        failure,
    } = evaluate_fresh(&stored, &git, &run, &context, executor)?;
    let status = match &fresh_snapshot {
        Some(snapshot) if snapshot.measurements == selection_snapshot.measurements => "matched",
        Some(_) => "drifted",
        None => "failed",
    };
    let evidence_path = verification_dir.join("verification.json");
    let evidence = VerificationEvidence {
        run_id: run_id.into(),
        verified_commit: run.head_commit().as_str().into(),
        selection_snapshot,
        fresh_snapshot,
        failed_evaluator: failure.as_ref().map(|(id, _)| id.clone()),
        failure: failure.map(|(_, failure)| failure),
        status: status.into(),
        evidence_path: evidence_path.clone(),
    };
    write_evidence(&evidence)?;
    Ok(evidence)
}

fn evaluate_fresh(
    stored: &StoredRun,
    git: &LockedGitRepository<'_>,
    run: &RunWorkspace,
    context: &EvaluationContext,
    executor: &impl BaselineExecutor,
) -> Result<FreshResult, RunnerError> {
    let started = Instant::now();
    let mut outputs = Vec::new();
    let mut failure = None;
    for evaluator in stored.manifest.evaluators() {
        let result = executor.evaluate(context, evaluator);
        if git.prepare_verification(run).is_err() {
            failure = Some((
                evaluator.id().to_owned(),
                EvaluatorFailure {
                    class: FailureClass::Validation,
                    detail: "verification worktree changed during evaluator".into(),
                },
            ));
            break;
        }
        match result {
            Ok(output) => outputs.push(output),
            Err(error) => {
                failure = Some((evaluator.id().to_owned(), redact_failure(&error)));
                break;
            }
        }
    }
    if git.open_run()?.head_commit() != run.head_commit() {
        return Err(RunnerError::InvalidState(
            "retained run ref moved during verification",
        ));
    }
    let fresh_snapshot = if failure.is_none() {
        if let Ok(snapshot) =
            build_snapshot(context, &stored.manifest, outputs, Complexity::default())
        {
            Some(FreshEvaluation {
                measurements: snapshot.measurements,
                runtime_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            })
        } else {
            failure = Some((
                "snapshot_validation".into(),
                EvaluatorFailure {
                    class: FailureClass::Validation,
                    detail: "fresh evaluator outputs did not form snapshot".into(),
                },
            ));
            None
        }
    } else {
        None
    };
    Ok(FreshResult {
        snapshot: fresh_snapshot,
        failure,
    })
}

fn write_evidence(evidence: &VerificationEvidence) -> Result<(), RunnerError> {
    let evidence_path = &evidence.evidence_path;
    let mut encoded = serde_json::to_vec_pretty(evidence)?;
    encoded.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(evidence_path)
        .map_err(|source| RunnerError::Io {
            path: evidence_path.clone(),
            source,
        })?;
    file.write_all(&encoded)
        .and_then(|()| file.sync_all())
        .map_err(|source| RunnerError::Io {
            path: evidence_path.clone(),
            source,
        })
}

fn selection_for_current(stored: &StoredRun) -> Result<EvaluationSnapshot, RunnerError> {
    stored
        .entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            JournalEvent::CandidateDecisionRecorded {
                candidate_commit,
                snapshot,
                decision,
                ..
            } if candidate_commit == stored.view.current_commit()
                && decision.disposition == Disposition::Keep =>
            {
                Some(snapshot.clone())
            }
            _ => None,
        })
        .ok_or(RunnerError::InvalidState(
            "kept commit has no frozen selection snapshot",
        ))
}

fn allocate_verification_directory(run_directory: &Path) -> Result<PathBuf, RunnerError> {
    let root = run_directory.join("verifications");
    ensure_real_directory(&root)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RunnerError::InvalidState("system clock precedes Unix epoch"))?
        .as_nanos();
    for suffix in 0_u16..1000 {
        let directory = root.join(format!("verify-{now}-{}-{suffix}", std::process::id()));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(RunnerError::Io {
                    path: directory,
                    source,
                });
            }
        }
    }
    Err(RunnerError::InvalidState(
        "could not allocate independent verification directory",
    ))
}
