//! Versioned deterministic reports from replayed run evidence. Market receipts
//! stay separate from internal code-selection measurements.

mod board;

pub use board::{BoardError, render_board};

use autoresearch_core::{
    CandidateDecision, CandidateFinalization, DecisionError, EvaluationSnapshot, EvaluatorFailure,
    JournalError, JournalEvent, Measurement, MetricDirection, NumericMetricKind, RecoveryAction,
    ReplayState, RepoPath, replay_journal, select_candidate,
};
use autoresearch_evaluator::EvaluatorOutput;
use autoresearch_market::MarketEvidenceLedger;
use autoresearch_runner::{EnvironmentRecord, ReportSource};
use serde::Serialize;
use thiserror::Error;

/// Stable report schema for readers and static board generation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunReportV1 {
    /// Schema version, currently 1.
    pub schema_version: u8,
    /// Exact frozen run identifier.
    pub run_id: String,
    /// Original repository commit.
    pub base_commit: String,
    /// Current retained best commit from replayed journal.
    pub current_best_commit: String,
    /// Frozen manifest, prompt, gate, and fixture identity.
    pub frozen_identity_sha256: String,
    /// Redaction-safe host and command-contract fingerprint, or unavailable.
    pub environment: EnvironmentEvidence,
    /// Frozen primary objective.
    pub objective: ObjectiveDefinition,
    /// Exact replay next action.
    pub recovery_action: String,
    /// Typed baseline snapshot, if evaluator completed.
    pub baseline: Option<BaselineEvidence>,
    /// Typed baseline crash, if baseline never became comparable.
    pub baseline_failure: Option<FailureSummary>,
    /// Current best typed snapshot, never market-derived.
    pub current_best_snapshot: Option<EvaluationSnapshot>,
    /// Ordered exact candidate timeline.
    pub candidates: Vec<CandidateEvidence>,
    /// Optional durable stop reason.
    pub stop_reason: Option<String>,
    /// Read-only receipts, never merged into gates or objectives.
    pub market_evidence: MarketEvidenceLedger,
    /// Fixed boundary statement; importer does not make promotion decisions.
    pub commercial_validation_status: String,
}

/// Host and frozen command names recorded before first evaluator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvironmentEvidence {
    /// Journal-bound fingerprint, absent in legacy run.
    pub fingerprint_sha256: Option<String>,
    /// Captured record, absent in legacy run.
    pub record: Option<EnvironmentRecord>,
    /// Captured or unavailable; never inferred from report host.
    pub status: String,
}

/// Frozen objective name and direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectiveDefinition {
    /// Frozen numeric name.
    pub name: String,
    /// Improvement direction.
    pub direction: MetricDirection,
}

/// Baseline snapshot and explicit artifact-availability boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineEvidence {
    /// Exact base commit.
    pub commit: String,
    /// Validated frozen evaluator measurements.
    pub snapshot: EvaluationSnapshot,
    /// Legacy baseline journal lacks individual artifact references.
    pub artifact_references_status: String,
}

/// One journal-prepared candidate, including incomplete states.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CandidateEvidence {
    /// One-based candidate number.
    pub index: u32,
    /// Exact parent current-best commit at preparation.
    pub parent_commit: String,
    /// Exact evaluated commit, if decision recorded.
    pub candidate_commit: Option<String>,
    /// Paths journaled from exact committed diff, empty on older runs.
    pub changed_paths: Vec<String>,
    /// Journaled or unavailable for legacy run.
    pub changed_paths_status: String,
    /// Frozen gate matrix against prior best, when evaluated.
    pub hard_gates: Vec<GateComparison>,
    /// Signed candidate-minus-prior-best primary objective change.
    pub objective_delta: Option<f64>,
    /// Candidate evaluation runtime recorded at selection.
    pub runtime_ms: Option<u64>,
    /// Typed evaluator snapshot, if decision recorded.
    pub snapshot: Option<EvaluationSnapshot>,
    /// Frozen keep/discard reason, if decided.
    pub decision: Option<CandidateDecision>,
    /// Confirmed retained-ref effect, if finalized.
    pub finalization: Option<CandidateFinalization>,
    /// Validated run-owned artifact references from evaluator envelopes.
    pub artifacts: Vec<ArtifactReference>,
    /// Failed evaluator attempts, never counted as measured results.
    pub failures: Vec<FailureSummary>,
    /// Prepared, decided, kept, or discarded.
    pub state: String,
}

/// Journaled redacted evaluator failure with exact adapter identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailureSummary {
    /// Frozen evaluator ID.
    pub evaluator_id: String,
    /// Typed failure; process output remains withheld by runner.
    pub failure: EvaluatorFailure,
}

/// Baseline or prior-best gate versus candidate gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateComparison {
    /// Frozen hard-gate name.
    pub name: String,
    /// Prior current-best outcome.
    pub prior_best_passed: bool,
    /// Candidate outcome, absent before evaluation.
    pub candidate_passed: Option<bool>,
}

/// Validated portable artifact reference; no raw process log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtifactReference {
    /// Frozen evaluator ID.
    pub evaluator_id: String,
    /// Artifact name.
    pub name: String,
    /// Path relative to run-owned artifact directory.
    pub relative_path: String,
    /// Declared MIME type.
    pub media_type: String,
}

/// Report cannot be trusted or deterministically serialized.
#[derive(Debug, Error)]
pub enum ReportError {
    /// Journal replay rejected malformed or contradictory history.
    #[error(transparent)]
    Journal(#[from] JournalError),
    /// Journaled decision disagrees with frozen selection policy.
    #[error(transparent)]
    Decision(#[from] DecisionError),
    /// Stored evaluator output cannot be parsed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Source or journal identity mismatch.
    #[error("report source invalid: {0}")]
    InvalidSource(&'static str),
    /// Objective subtraction became nonfinite.
    #[error("candidate objective delta is not finite")]
    NonFiniteDelta,
}

/// Builds stable report from runner-validated frozen source and separately
/// imported receipts. Replays journal again so malformed manual sources fail
/// closed. No report field can alter runner decision or product gate.
///
/// # Errors
///
/// Rejects malformed journal, mismatched frozen identity, invalid captured
/// evaluator envelope, unsafe artifact path, or nonfinite objective delta.
pub fn build_report(
    source: &ReportSource,
    market_evidence: MarketEvidenceLedger,
) -> Result<RunReportV1, ReportError> {
    let ReplayState::Run(view) = replay_journal(&source.entries)? else {
        return Err(ReportError::InvalidSource("journal is empty"));
    };
    if view.run_id() != source.run_id
        || view.base_commit() != source.base_commit
        || view.current_commit() != source.current_commit
        || view.frozen_identity() != source.identity.aggregate_sha256
    {
        return Err(ReportError::InvalidSource(
            "journal differs from loaded frozen source",
        ));
    }
    validate_environment(source, &view)?;
    if let Some(snapshot) = view.baseline() {
        validate_measurements(source, snapshot)?;
    }
    let baseline = view.baseline().cloned().map(|snapshot| BaselineEvidence {
        commit: source.base_commit.clone(),
        snapshot,
        artifact_references_status: "not_recorded_in_baseline_journal".into(),
    });
    let (candidates, current_best_snapshot) = build_timeline(source, view.baseline())?;
    let objective = source.manifest.experiment().objective();
    Ok(RunReportV1 {
        schema_version: 1,
        run_id: source.run_id.clone(),
        base_commit: source.base_commit.clone(),
        current_best_commit: source.current_commit.clone(),
        frozen_identity_sha256: source.identity.aggregate_sha256.clone(),
        environment: EnvironmentEvidence {
            fingerprint_sha256: source.environment_fingerprint.clone(),
            record: source.environment.clone(),
            status: if source.environment.is_some() {
                "captured".into()
            } else {
                "unavailable_in_legacy_run".into()
            },
        },
        objective: ObjectiveDefinition {
            name: objective.name().into(),
            direction: objective.direction(),
        },
        recovery_action: action_name(view.recovery_action()).into(),
        baseline,
        baseline_failure: view
            .baseline_failure()
            .map(|(evaluator_id, failure)| FailureSummary {
                evaluator_id: evaluator_id.clone(),
                failure: failure.clone(),
            }),
        current_best_snapshot,
        candidates,
        stop_reason: view.stop_reason().map(str::to_owned),
        market_evidence,
        commercial_validation_status: "not_assessed_by_internal_evaluators".into(),
    })
}

/// Emits deterministic pretty JSON with one final newline.
///
/// # Errors
///
/// Returns serialization failure.
pub fn to_json_bytes(report: &RunReportV1) -> Result<Vec<u8>, ReportError> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate_environment(
    source: &ReportSource,
    view: &autoresearch_core::RunView,
) -> Result<(), ReportError> {
    if source.environment_fingerprint.as_deref() != view.environment_fingerprint() {
        return Err(ReportError::InvalidSource(
            "environment fingerprint differs from journal",
        ));
    }
    match (&source.environment, &source.environment_fingerprint) {
        (Some(record), Some(fingerprint)) => {
            let (_, computed) = record.encoded_fingerprint()?;
            if &computed != fingerprint || !record.matches_manifest(&source.manifest) {
                return Err(ReportError::InvalidSource("environment record is invalid"));
            }
        }
        (None, None) => {}
        _ => {
            return Err(ReportError::InvalidSource(
                "environment record is incomplete",
            ));
        }
    }
    Ok(())
}

fn action_name(action: &RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::StartRun => "start_run",
        RecoveryAction::CaptureBaseline => "capture_baseline",
        RecoveryAction::PrepareCandidate { .. } => "prepare_candidate",
        RecoveryAction::EvaluateCandidate { .. } => "evaluate_candidate",
        RecoveryAction::FinalizeCandidate { .. } => "finalize_candidate",
        RecoveryAction::Finished => "finished",
    }
}

fn build_timeline(
    source: &ReportSource,
    baseline: Option<&EvaluationSnapshot>,
) -> Result<(Vec<CandidateEvidence>, Option<EvaluationSnapshot>), ReportError> {
    let mut candidates = Vec::new();
    let mut current_best = baseline.cloned();
    let mut prior_best = None;
    for entry in &source.entries {
        match &entry.event {
            JournalEvent::CandidatePrepared {
                index,
                parent_commit,
                ..
            } => {
                prior_best.clone_from(&current_best);
                candidates.push(prepared_candidate(
                    *index,
                    parent_commit,
                    prior_best.as_ref(),
                ));
            }
            JournalEvent::CandidateEvaluatorFailed {
                evaluator_id,
                failure,
                ..
            } => {
                let candidate = candidates.last_mut().ok_or(ReportError::InvalidSource(
                    "failed evaluator has no prepared candidate",
                ))?;
                candidate.failures.push(FailureSummary {
                    evaluator_id: evaluator_id.clone(),
                    failure: failure.clone(),
                });
            }
            JournalEvent::CandidateEvaluatorCaptured {
                evaluator_id,
                evaluated_commit,
                output_json,
                ..
            } => {
                let candidate = candidates.last_mut().ok_or(ReportError::InvalidSource(
                    "evaluator output has no prepared candidate",
                ))?;
                candidate.artifacts.extend(artifact_references(
                    source,
                    evaluator_id,
                    evaluated_commit,
                    output_json,
                )?);
            }
            JournalEvent::CandidateDecisionRecorded {
                candidate_commit,
                changed_paths,
                snapshot,
                decision,
                ..
            } => {
                let candidate = candidates.last_mut().ok_or(ReportError::InvalidSource(
                    "decision has no prepared candidate",
                ))?;
                validate_candidate_decision(source, prior_best.as_ref(), snapshot, decision)?;
                candidate.candidate_commit = Some(candidate_commit.clone());
                candidate.changed_paths.clone_from(changed_paths);
                candidate.changed_paths_status = if changed_paths.is_empty() {
                    "unavailable_in_legacy_journal"
                } else {
                    "journaled"
                }
                .into();
                candidate.hard_gates = gate_matrix(prior_best.as_ref(), Some(snapshot));
                candidate.objective_delta = objective_delta(
                    prior_best.as_ref(),
                    snapshot,
                    source.manifest.experiment().objective().name(),
                )?;
                candidate.runtime_ms = Some(snapshot.complexity.runtime_ms);
                candidate.snapshot = Some(snapshot.clone());
                candidate.decision = Some(decision.clone());
                candidate.state = "decided".into();
            }
            JournalEvent::CandidateFinalized { outcome, .. } => {
                let candidate = candidates.last_mut().ok_or(ReportError::InvalidSource(
                    "finalization has no prepared candidate",
                ))?;
                candidate.finalization = Some(outcome.clone());
                if let CandidateFinalization::Kept { .. } = outcome {
                    current_best.clone_from(&candidate.snapshot);
                    candidate.state = "kept".into();
                } else {
                    candidate.state = "discarded".into();
                }
            }
            _ => {}
        }
    }
    Ok((candidates, current_best))
}

fn prepared_candidate(
    index: u32,
    parent_commit: &str,
    prior_best: Option<&EvaluationSnapshot>,
) -> CandidateEvidence {
    CandidateEvidence {
        index,
        parent_commit: parent_commit.into(),
        candidate_commit: None,
        changed_paths: Vec::new(),
        changed_paths_status: "not_yet_recorded".into(),
        hard_gates: gate_matrix(prior_best, None),
        objective_delta: None,
        runtime_ms: None,
        snapshot: None,
        decision: None,
        finalization: None,
        artifacts: Vec::new(),
        failures: Vec::new(),
        state: "prepared".into(),
    }
}

fn validate_candidate_decision(
    source: &ReportSource,
    prior_best: Option<&EvaluationSnapshot>,
    snapshot: &EvaluationSnapshot,
    decision: &CandidateDecision,
) -> Result<(), ReportError> {
    validate_measurements(source, snapshot)?;
    let prior = prior_best.ok_or(ReportError::InvalidSource(
        "candidate decision has no prior best snapshot",
    ))?;
    let expected = select_candidate(
        prior,
        snapshot,
        source.manifest.experiment().objective().name(),
    )?;
    if &expected != decision {
        return Err(ReportError::InvalidSource(
            "journaled decision differs from frozen policy",
        ));
    }
    Ok(())
}

fn validate_measurements(
    source: &ReportSource,
    snapshot: &EvaluationSnapshot,
) -> Result<(), ReportError> {
    let mut expected = Vec::new();
    for evaluator in source.manifest.evaluators() {
        for gate in evaluator.hard_gates() {
            expected.push((gate.as_str(), None));
        }
        for metric in evaluator.metrics() {
            expected.push((metric.name(), Some((metric.kind(), metric.direction()))));
        }
    }
    if snapshot.measurements.len() != expected.len() {
        return Err(ReportError::InvalidSource(
            "snapshot measurement count differs from frozen manifest",
        ));
    }
    for (measurement, (name, numeric)) in snapshot.measurements.iter().zip(expected) {
        let valid = match (measurement, numeric) {
            (Measurement::HardGate { name: actual, .. }, None) => actual == name,
            (
                Measurement::Numeric {
                    name: actual,
                    metric_kind,
                    direction,
                    ..
                },
                Some((kind, expected_direction)),
            ) => actual == name && *metric_kind == kind && *direction == expected_direction,
            _ => false,
        };
        if !valid {
            return Err(ReportError::InvalidSource(
                "snapshot measurement differs from frozen manifest",
            ));
        }
    }
    Ok(())
}

fn gate_matrix(
    prior: Option<&EvaluationSnapshot>,
    candidate: Option<&EvaluationSnapshot>,
) -> Vec<GateComparison> {
    let Some(prior) = prior else {
        return Vec::new();
    };
    prior
        .measurements
        .iter()
        .filter_map(|measurement| {
            let Measurement::HardGate { name, outcome } = measurement else {
                return None;
            };
            let candidate_passed = candidate.and_then(|snapshot| {
                snapshot.measurements.iter().find_map(|item| match item {
                    Measurement::HardGate {
                        name: candidate_name,
                        outcome,
                    } if candidate_name == name => Some(outcome.passed()),
                    _ => None,
                })
            });
            Some(GateComparison {
                name: name.clone(),
                prior_best_passed: outcome.passed(),
                candidate_passed,
            })
        })
        .collect()
}

fn objective_delta(
    prior: Option<&EvaluationSnapshot>,
    candidate: &EvaluationSnapshot,
    objective_name: &str,
) -> Result<Option<f64>, ReportError> {
    let Some(prior) = prior else { return Ok(None) };
    let score = |snapshot: &EvaluationSnapshot| {
        snapshot
            .measurements
            .iter()
            .find_map(|measurement| match measurement {
                Measurement::Numeric {
                    name,
                    metric_kind: NumericMetricKind::Objective,
                    value,
                    ..
                } if name == objective_name => Some(value.get()),
                _ => None,
            })
    };
    let (Some(before), Some(after)) = (score(prior), score(candidate)) else {
        return Err(ReportError::InvalidSource(
            "primary objective is missing from snapshot",
        ));
    };
    let delta = after - before;
    if !delta.is_finite() {
        return Err(ReportError::NonFiniteDelta);
    }
    Ok(Some(delta))
}

fn artifact_references(
    source: &ReportSource,
    evaluator_id: &str,
    evaluated_commit: &str,
    output_json: &str,
) -> Result<Vec<ArtifactReference>, ReportError> {
    let output: EvaluatorOutput = serde_json::from_str(output_json)?;
    if output.evaluator_id != evaluator_id
        || output.run_id != source.run_id
        || output.baseline_commit != source.base_commit
        || output.evaluated_commit != evaluated_commit
        || source
            .manifest
            .evaluators()
            .iter()
            .all(|item| item.id() != evaluator_id)
        || output
            .measurements
            .iter()
            .any(|measurement| measurement.kind() == autoresearch_core::MetricKind::MarketEvidence)
    {
        return Err(ReportError::InvalidSource(
            "captured evaluator output identity or kind is invalid",
        ));
    }
    output
        .artifacts
        .into_iter()
        .map(|artifact| {
            RepoPath::new(&artifact.relative_path)
                .map_err(|_| ReportError::InvalidSource("captured artifact path is unsafe"))?;
            Ok(ArtifactReference {
                evaluator_id: evaluator_id.into(),
                name: artifact.name,
                relative_path: artifact.relative_path,
                media_type: artifact.media_type,
            })
        })
        .collect()
}
