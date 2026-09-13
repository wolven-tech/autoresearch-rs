//! Append-only run events and pure crash-recovery replay.

use crate::{CandidateDecision, Disposition, EvaluationSnapshot, EvaluatorFailure, RepoPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// Sequence-numbered event stored as one journal record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Zero-based, gap-free sequence number.
    pub sequence: u64,
    /// Stable run identifier repeated on every record.
    pub run_id: String,
    /// State transition payload.
    pub event: JournalEvent,
}

/// Durable run transition written before associated side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JournalEvent {
    /// Creates run identity and fixes starting revision.
    RunStarted {
        /// Validated Git commit used by baseline.
        base_commit: String,
        /// Aggregate frozen-input digest.
        frozen_identity: String,
    },
    /// Binds redaction-safe local runtime facts captured before evaluation.
    EnvironmentCaptured {
        /// SHA-256 of run-owned environment.json bytes.
        fingerprint_sha256: String,
    },
    /// Stores comparable baseline evaluation.
    BaselineCaptured {
        /// Typed evaluator output for base commit.
        snapshot: EvaluationSnapshot,
    },
    /// Stops before mutation when a declared baseline evaluator cannot run.
    BaselineFailed {
        /// Frozen evaluator that failed.
        evaluator_id: String,
        /// Typed, redacted evaluator failure; never a fabricated score.
        failure: EvaluatorFailure,
    },
    /// Declares isolated candidate before mutation command runs.
    CandidatePrepared {
        /// One-based candidate number.
        index: u32,
        /// Current-best commit from which candidate starts.
        parent_commit: String,
        /// Relocation-independent worktree identifier.
        worktree_id: String,
    },
    /// Stores one completed declared evaluator output before next evaluator.
    /// Runner revalidates opaque JSON against SDK and frozen manifest on replay.
    CandidateEvaluatorCaptured {
        /// Active one-based candidate number.
        index: u32,
        /// Frozen evaluator ID.
        evaluator_id: String,
        /// Exact candidate commit evaluated.
        evaluated_commit: String,
        /// Bounded complete evaluator output envelope.
        output_json: String,
    },
    /// Records failed evaluator attempt without fabricating a snapshot or
    /// changing next recovery action; later resume may retry exact candidate.
    CandidateEvaluatorFailed {
        /// Active one-based candidate number.
        index: u32,
        /// Frozen evaluator ID.
        evaluator_id: String,
        /// Exact candidate commit attempted.
        evaluated_commit: String,
        /// Typed bounded failure summary.
        failure: EvaluatorFailure,
    },
    /// Stores evaluation and selection before Git finalization.
    CandidateDecisionRecorded {
        /// Candidate number matching preparation record.
        index: u32,
        /// Exact candidate commit evaluated by frozen policy.
        candidate_commit: String,
        /// Canonical changed paths from exact committed candidate diff.
        /// Empty only in journals created before this field existed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        changed_paths: Vec<String>,
        /// Typed evaluator output.
        snapshot: EvaluationSnapshot,
        /// Frozen-policy result.
        decision: CandidateDecision,
    },
    /// Confirms keep/discard side effect completed.
    CandidateFinalized {
        /// Candidate number matching active candidate.
        index: u32,
        /// Applied outcome.
        outcome: CandidateFinalization,
    },
    /// Closes run after all candidate side effects settle.
    RunStopped {
        /// Stable operator or budget reason.
        reason: String,
    },
}

/// Side-effect result for one candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CandidateFinalization {
    /// Candidate advanced current-best branch to commit.
    Kept {
        /// Resulting commit identifier.
        commit: String,
    },
    /// Candidate worktree was safely discarded.
    Discarded,
}

/// Pure result of replaying zero or more journal entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayState {
    /// No durable run exists.
    Empty,
    /// Valid durable run and exact next operation.
    Run(Box<RunView>),
}

impl ReplayState {
    /// Returns operation required to advance or recover run.
    #[must_use]
    pub fn recovery_action(&self) -> RecoveryAction {
        match self {
            Self::Empty => RecoveryAction::StartRun,
            Self::Run(view) => view.recovery_action.clone(),
        }
    }
}

/// Replayed durable run projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    run_id: String,
    frozen_identity: String,
    environment_fingerprint: Option<String>,
    base_commit: String,
    current_commit: String,
    baseline: Option<EvaluationSnapshot>,
    baseline_failure: Option<(String, EvaluatorFailure)>,
    completed_candidates: u32,
    stop_reason: Option<String>,
    recovery_action: RecoveryAction,
}

impl RunView {
    /// Returns stable run identifier.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Returns aggregate frozen-input digest.
    #[must_use]
    pub fn frozen_identity(&self) -> &str {
        &self.frozen_identity
    }

    /// Returns captured environment-record digest, if this run records one.
    #[must_use]
    pub fn environment_fingerprint(&self) -> Option<&str> {
        self.environment_fingerprint.as_deref()
    }

    /// Returns original baseline commit.
    #[must_use]
    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }

    /// Returns latest kept commit, or base commit before first keep.
    #[must_use]
    pub fn current_commit(&self) -> &str {
        &self.current_commit
    }

    /// Returns captured baseline when available.
    #[must_use]
    pub const fn baseline(&self) -> Option<&EvaluationSnapshot> {
        self.baseline.as_ref()
    }

    /// Returns typed baseline evaluator failure when run stopped before mutation.
    #[must_use]
    pub const fn baseline_failure(&self) -> Option<&(String, EvaluatorFailure)> {
        self.baseline_failure.as_ref()
    }

    /// Returns number of fully finalized candidates.
    #[must_use]
    pub const fn completed_candidates(&self) -> u32 {
        self.completed_candidates
    }

    /// Returns stop reason for completed run.
    #[must_use]
    pub fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }

    /// Returns exact next recovery operation.
    #[must_use]
    pub const fn recovery_action(&self) -> &RecoveryAction {
        &self.recovery_action
    }
}

/// Next operation inferred from last durable transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Create first run-start record.
    StartRun,
    /// Execute and record baseline evaluators.
    CaptureBaseline,
    /// Create next isolated candidate from current best.
    PrepareCandidate {
        /// Expected one-based candidate number.
        index: u32,
        /// Commit from which candidate must branch.
        parent_commit: String,
    },
    /// Run mutation and evaluators for prepared candidate.
    EvaluateCandidate {
        /// Prepared candidate number.
        index: u32,
        /// Relocation-independent worktree identifier.
        worktree_id: String,
    },
    /// Apply previously recorded decision and journal finalization.
    FinalizeCandidate {
        /// Evaluated candidate number.
        index: u32,
        /// Relocation-independent worktree identifier.
        worktree_id: String,
        /// Exact candidate commit whose decision must be applied.
        candidate_commit: String,
        /// Durable decision that must be applied exactly.
        decision: CandidateDecision,
    },
    /// Run has stopped and cannot accept more events.
    Finished,
}

/// Invalid or ambiguous journal history.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JournalError {
    /// Sequence must start at zero and remain gap-free.
    #[error("journal sequence mismatch: expected {expected}, got {actual}")]
    Sequence {
        /// Required sequence.
        expected: u64,
        /// Observed sequence.
        actual: u64,
    },
    /// Each record must carry one stable run identifier.
    #[error("journal run id mismatch: expected `{expected}`, got `{actual}`")]
    RunIdMismatch {
        /// First record run identifier.
        expected: String,
        /// Conflicting record run identifier.
        actual: String,
    },
    /// Required identifier or reason is blank.
    #[error("{0} cannot be blank")]
    Blank(&'static str),
    /// Non-start event appeared before run creation.
    #[error("journal must begin with run_started")]
    MissingRunStart,
    /// Second run-start event appeared in same journal.
    #[error("run_started may appear only once")]
    DuplicateRunStart,
    /// Event does not apply in current state.
    #[error("event `{event}` is invalid while run is `{state}`")]
    InvalidTransition {
        /// Event label.
        event: &'static str,
        /// Current state label.
        state: &'static str,
    },
    /// Candidates must advance one at a time.
    #[error("candidate index mismatch: expected {expected}, got {actual}")]
    CandidateIndex {
        /// Required next candidate number.
        expected: u32,
        /// Observed candidate number.
        actual: u32,
    },
    /// Prepared candidate must branch from current best.
    #[error("candidate parent mismatch: expected `{expected}`, got `{actual}`")]
    ParentCommitMismatch {
        /// Current-best commit.
        expected: String,
        /// Declared parent commit.
        actual: String,
    },
    /// Finalized side effect must implement recorded decision.
    #[error("candidate finalization contradicts recorded decision")]
    FinalizationMismatch,
    /// Same evaluator cannot complete twice for one candidate.
    #[error("candidate evaluator `{0}` was captured more than once")]
    DuplicateEvaluator(String),
    /// All evaluator outputs and final decision must identify one commit.
    #[error("candidate evaluator commit differs from decision commit")]
    EvaluatorCommitMismatch,
    /// Captured output exceeded bounded journal record size.
    #[error("candidate evaluator output exceeds byte limit")]
    EvaluatorOutputTooLarge,
    /// Candidate count exceeded representable range.
    #[error("candidate index overflow")]
    CandidateOverflow,
}

/// Replays complete append-only journal into exact recovery state.
///
/// # Errors
///
/// Returns [`JournalError`] when entries contain gaps, cross-run records,
/// illegal transitions, mismatched candidates, or contradictory finalization.
pub fn replay_journal(entries: &[JournalEntry]) -> Result<ReplayState, JournalError> {
    if entries.is_empty() {
        return Ok(ReplayState::Empty);
    }

    let run_id = checked(&entries[0].run_id, "run_id")?.to_owned();
    let mut machine = Machine::new(run_id.clone());
    let mut expected_sequence = 0_u64;
    for entry in entries {
        if entry.sequence != expected_sequence {
            return Err(JournalError::Sequence {
                expected: expected_sequence,
                actual: entry.sequence,
            });
        }
        expected_sequence = expected_sequence.saturating_add(1);
        if entry.run_id != run_id {
            return Err(JournalError::RunIdMismatch {
                expected: run_id.clone(),
                actual: entry.run_id.clone(),
            });
        }
        machine.apply(&entry.event)?;
    }

    Ok(ReplayState::Run(Box::new(machine.finish()?)))
}

#[derive(Debug, Clone)]
enum CandidateStage {
    Prepared {
        index: u32,
        worktree_id: String,
    },
    Decided {
        index: u32,
        worktree_id: String,
        candidate_commit: String,
        decision: CandidateDecision,
    },
}

struct Machine {
    run_id: String,
    frozen_identity: Option<String>,
    environment_fingerprint: Option<String>,
    base_commit: Option<String>,
    current_commit: Option<String>,
    baseline: Option<EvaluationSnapshot>,
    baseline_failure: Option<(String, EvaluatorFailure)>,
    active: Option<CandidateStage>,
    evaluator_commit: Option<String>,
    completed_evaluators: BTreeSet<String>,
    completed_candidates: u32,
    stop_reason: Option<String>,
}

impl Machine {
    fn new(run_id: String) -> Self {
        Self {
            run_id,
            frozen_identity: None,
            environment_fingerprint: None,
            base_commit: None,
            current_commit: None,
            baseline: None,
            baseline_failure: None,
            active: None,
            evaluator_commit: None,
            completed_evaluators: BTreeSet::new(),
            completed_candidates: 0,
            stop_reason: None,
        }
    }

    fn state_name(&self) -> &'static str {
        if self.stop_reason.is_some() {
            "stopped"
        } else if matches!(self.active, Some(CandidateStage::Decided { .. })) {
            "candidate_decided"
        } else if matches!(self.active, Some(CandidateStage::Prepared { .. })) {
            "candidate_prepared"
        } else if self.baseline.is_some() {
            "baseline_captured"
        } else if self.base_commit.is_some() {
            "started"
        } else {
            "empty"
        }
    }

    fn apply(&mut self, event: &JournalEvent) -> Result<(), JournalError> {
        if self.stop_reason.is_some() {
            return Err(JournalError::InvalidTransition {
                event: event_name(event),
                state: "stopped",
            });
        }

        match event {
            JournalEvent::RunStarted {
                base_commit,
                frozen_identity,
            } => self.start(base_commit, frozen_identity),
            JournalEvent::EnvironmentCaptured { fingerprint_sha256 } => {
                self.capture_environment(fingerprint_sha256)
            }
            JournalEvent::BaselineCaptured { snapshot } => self.capture_baseline(snapshot),
            JournalEvent::BaselineFailed {
                evaluator_id,
                failure,
            } => self.fail_baseline(evaluator_id, failure),
            JournalEvent::CandidatePrepared {
                index,
                parent_commit,
                worktree_id,
            } => self.prepare(*index, parent_commit, worktree_id),
            JournalEvent::CandidateEvaluatorCaptured {
                index,
                evaluator_id,
                evaluated_commit,
                output_json,
            } => self.capture_evaluator(*index, evaluator_id, evaluated_commit, output_json),
            JournalEvent::CandidateEvaluatorFailed {
                index,
                evaluator_id,
                evaluated_commit,
                failure,
            } => self.fail_evaluator(*index, evaluator_id, evaluated_commit, failure),
            JournalEvent::CandidateDecisionRecorded {
                index,
                candidate_commit,
                changed_paths,
                snapshot,
                decision,
            } => self.record_decision(*index, candidate_commit, changed_paths, snapshot, decision),
            JournalEvent::CandidateFinalized { index, outcome } => self.finalize(*index, outcome),
            JournalEvent::RunStopped { reason } => self.stop(reason),
        }
    }

    fn start(&mut self, base_commit: &str, frozen_identity: &str) -> Result<(), JournalError> {
        if self.base_commit.is_some() {
            return Err(JournalError::DuplicateRunStart);
        }
        let base_commit = checked(base_commit, "base_commit")?.to_owned();
        let frozen_identity = checked(frozen_identity, "frozen_identity")?.to_owned();
        self.current_commit = Some(base_commit.clone());
        self.base_commit = Some(base_commit);
        self.frozen_identity = Some(frozen_identity);
        Ok(())
    }

    fn capture_environment(&mut self, fingerprint_sha256: &str) -> Result<(), JournalError> {
        if self.base_commit.is_none()
            || self.environment_fingerprint.is_some()
            || self.baseline.is_some()
            || self.active.is_some()
        {
            return Err(self.invalid("environment_captured"));
        }
        if fingerprint_sha256.len() != 64
            || !fingerprint_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(self.invalid("environment_captured"));
        }
        self.environment_fingerprint = Some(fingerprint_sha256.into());
        Ok(())
    }

    fn capture_baseline(&mut self, snapshot: &EvaluationSnapshot) -> Result<(), JournalError> {
        if self.base_commit.is_none() || self.baseline.is_some() || self.active.is_some() {
            return Err(self.invalid("baseline_captured"));
        }
        self.baseline = Some(snapshot.clone());
        Ok(())
    }

    fn fail_baseline(
        &mut self,
        evaluator_id: &str,
        failure: &EvaluatorFailure,
    ) -> Result<(), JournalError> {
        if self.base_commit.is_none() || self.baseline.is_some() || self.active.is_some() {
            return Err(self.invalid("baseline_failed"));
        }
        let evaluator_id = checked(evaluator_id, "evaluator_id")?.to_owned();
        checked(&failure.detail, "baseline_failure_detail")?;
        self.baseline_failure = Some((evaluator_id, failure.clone()));
        self.stop_reason = Some("baseline_evaluator_failure".into());
        Ok(())
    }

    fn prepare(
        &mut self,
        index: u32,
        parent_commit: &str,
        worktree_id: &str,
    ) -> Result<(), JournalError> {
        if self.baseline.is_none() || self.active.is_some() {
            return Err(self.invalid("candidate_prepared"));
        }
        let expected = self.next_candidate_index()?;
        if index != expected {
            return Err(JournalError::CandidateIndex {
                expected,
                actual: index,
            });
        }
        let current_commit = self
            .current_commit
            .as_deref()
            .ok_or(JournalError::MissingRunStart)?;
        if parent_commit != current_commit {
            return Err(JournalError::ParentCommitMismatch {
                expected: current_commit.to_owned(),
                actual: parent_commit.to_owned(),
            });
        }
        let worktree_id = checked(worktree_id, "worktree_id")?.to_owned();
        self.active = Some(CandidateStage::Prepared { index, worktree_id });
        self.evaluator_commit = None;
        self.completed_evaluators.clear();
        Ok(())
    }

    fn capture_evaluator(
        &mut self,
        index: u32,
        evaluator_id: &str,
        evaluated_commit: &str,
        output_json: &str,
    ) -> Result<(), JournalError> {
        let Some(CandidateStage::Prepared { index: active, .. }) = &self.active else {
            return Err(self.invalid("candidate_evaluator_captured"));
        };
        if index != *active {
            return Err(JournalError::CandidateIndex {
                expected: *active,
                actual: index,
            });
        }
        checked(evaluator_id, "evaluator_id")?;
        checked(evaluated_commit, "evaluated_commit")?;
        checked(output_json, "output_json")?;
        if output_json.len() > 1024 * 1024 {
            return Err(JournalError::EvaluatorOutputTooLarge);
        }
        if self
            .evaluator_commit
            .as_ref()
            .is_some_and(|commit| commit != evaluated_commit)
        {
            return Err(JournalError::EvaluatorCommitMismatch);
        }
        if !self.completed_evaluators.insert(evaluator_id.into()) {
            return Err(JournalError::DuplicateEvaluator(evaluator_id.into()));
        }
        self.evaluator_commit = Some(evaluated_commit.into());
        Ok(())
    }

    fn fail_evaluator(
        &mut self,
        index: u32,
        evaluator_id: &str,
        evaluated_commit: &str,
        failure: &EvaluatorFailure,
    ) -> Result<(), JournalError> {
        let Some(CandidateStage::Prepared { index: active, .. }) = &self.active else {
            return Err(self.invalid("candidate_evaluator_failed"));
        };
        if index != *active {
            return Err(JournalError::CandidateIndex {
                expected: *active,
                actual: index,
            });
        }
        checked(evaluator_id, "evaluator_id")?;
        checked(evaluated_commit, "evaluated_commit")?;
        checked(&failure.detail, "candidate_failure_detail")?;
        if self
            .evaluator_commit
            .as_ref()
            .is_some_and(|commit| commit != evaluated_commit)
            || self.completed_evaluators.contains(evaluator_id)
        {
            return Err(JournalError::EvaluatorCommitMismatch);
        }
        self.evaluator_commit = Some(evaluated_commit.into());
        Ok(())
    }

    fn record_decision(
        &mut self,
        index: u32,
        candidate_commit: &str,
        changed_paths: &[String],
        _snapshot: &EvaluationSnapshot,
        decision: &CandidateDecision,
    ) -> Result<(), JournalError> {
        let mut prior = None;
        for path in changed_paths {
            if RepoPath::new(path).is_err() || prior.is_some_and(|value: &str| value >= path) {
                return Err(self.invalid("candidate_decision_recorded"));
            }
            prior = Some(path);
        }
        let Some(CandidateStage::Prepared {
            index: active_index,
            worktree_id,
        }) = self.active.take()
        else {
            return Err(self.invalid("candidate_decision_recorded"));
        };
        if index != active_index {
            self.active = Some(CandidateStage::Prepared {
                index: active_index,
                worktree_id,
            });
            return Err(JournalError::CandidateIndex {
                expected: active_index,
                actual: index,
            });
        }
        if self
            .evaluator_commit
            .as_ref()
            .is_some_and(|commit| commit != candidate_commit)
        {
            return Err(JournalError::EvaluatorCommitMismatch);
        }
        self.active = Some(CandidateStage::Decided {
            index,
            worktree_id,
            candidate_commit: checked(candidate_commit, "candidate_commit")?.to_owned(),
            decision: decision.clone(),
        });
        Ok(())
    }

    fn finalize(
        &mut self,
        index: u32,
        outcome: &CandidateFinalization,
    ) -> Result<(), JournalError> {
        let Some(CandidateStage::Decided {
            index: active_index,
            worktree_id,
            candidate_commit,
            decision,
        }) = self.active.take()
        else {
            return Err(self.invalid("candidate_finalized"));
        };
        if index != active_index {
            self.active = Some(CandidateStage::Decided {
                index: active_index,
                worktree_id,
                candidate_commit,
                decision,
            });
            return Err(JournalError::CandidateIndex {
                expected: active_index,
                actual: index,
            });
        }

        match (decision.disposition, outcome) {
            (Disposition::Keep, CandidateFinalization::Kept { commit })
                if commit == &candidate_commit =>
            {
                self.current_commit = Some(checked(commit, "kept_commit")?.to_owned());
            }
            (Disposition::Discard, CandidateFinalization::Discarded) => {}
            _ => {
                self.active = Some(CandidateStage::Decided {
                    index: active_index,
                    worktree_id,
                    candidate_commit,
                    decision,
                });
                return Err(JournalError::FinalizationMismatch);
            }
        }
        self.completed_candidates = index;
        self.evaluator_commit = None;
        self.completed_evaluators.clear();
        Ok(())
    }

    fn stop(&mut self, reason: &str) -> Result<(), JournalError> {
        if self.baseline.is_none() || self.active.is_some() {
            return Err(self.invalid("run_stopped"));
        }
        self.stop_reason = Some(checked(reason, "stop_reason")?.to_owned());
        Ok(())
    }

    fn next_candidate_index(&self) -> Result<u32, JournalError> {
        self.completed_candidates
            .checked_add(1)
            .ok_or(JournalError::CandidateOverflow)
    }

    fn invalid(&self, event: &'static str) -> JournalError {
        JournalError::InvalidTransition {
            event,
            state: self.state_name(),
        }
    }

    fn finish(self) -> Result<RunView, JournalError> {
        let Some(base_commit) = self.base_commit else {
            return Err(JournalError::MissingRunStart);
        };
        let Some(frozen_identity) = self.frozen_identity else {
            return Err(JournalError::MissingRunStart);
        };
        let current_commit = self.current_commit.ok_or(JournalError::MissingRunStart)?;
        let recovery_action = if self.stop_reason.is_some() {
            RecoveryAction::Finished
        } else {
            match &self.active {
                Some(CandidateStage::Prepared { index, worktree_id }) => {
                    RecoveryAction::EvaluateCandidate {
                        index: *index,
                        worktree_id: worktree_id.clone(),
                    }
                }
                Some(CandidateStage::Decided {
                    index,
                    worktree_id,
                    candidate_commit,
                    decision,
                }) => RecoveryAction::FinalizeCandidate {
                    index: *index,
                    worktree_id: worktree_id.clone(),
                    candidate_commit: candidate_commit.clone(),
                    decision: decision.clone(),
                },
                None if self.baseline.is_some() => RecoveryAction::PrepareCandidate {
                    index: self
                        .completed_candidates
                        .checked_add(1)
                        .ok_or(JournalError::CandidateOverflow)?,
                    parent_commit: current_commit.clone(),
                },
                None => RecoveryAction::CaptureBaseline,
            }
        };

        Ok(RunView {
            run_id: self.run_id,
            frozen_identity,
            environment_fingerprint: self.environment_fingerprint,
            base_commit,
            current_commit,
            baseline: self.baseline,
            baseline_failure: self.baseline_failure,
            completed_candidates: self.completed_candidates,
            stop_reason: self.stop_reason,
            recovery_action,
        })
    }
}

fn checked<'a>(value: &'a str, field: &'static str) -> Result<&'a str, JournalError> {
    if value.trim().is_empty() {
        Err(JournalError::Blank(field))
    } else {
        Ok(value)
    }
}

const fn event_name(event: &JournalEvent) -> &'static str {
    match event {
        JournalEvent::RunStarted { .. } => "run_started",
        JournalEvent::EnvironmentCaptured { .. } => "environment_captured",
        JournalEvent::BaselineCaptured { .. } => "baseline_captured",
        JournalEvent::BaselineFailed { .. } => "baseline_failed",
        JournalEvent::CandidatePrepared { .. } => "candidate_prepared",
        JournalEvent::CandidateEvaluatorCaptured { .. } => "candidate_evaluator_captured",
        JournalEvent::CandidateEvaluatorFailed { .. } => "candidate_evaluator_failed",
        JournalEvent::CandidateDecisionRecorded { .. } => "candidate_decision_recorded",
        JournalEvent::CandidateFinalized { .. } => "candidate_finalized",
        JournalEvent::RunStopped { .. } => "run_stopped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Complexity, DecisionReason, FailureClass, Measurement, MetricDirection, NumericMetricKind,
    };

    fn snapshot(value: f64) -> EvaluationSnapshot {
        EvaluationSnapshot {
            measurements: vec![
                Measurement::hard_gate("tests", true, None).expect("fixture"),
                Measurement::numeric(
                    "score",
                    NumericMetricKind::Objective,
                    MetricDirection::Maximize,
                    value,
                )
                .expect("fixture"),
            ],
            complexity: Complexity::default(),
        }
    }

    fn decision(disposition: Disposition) -> CandidateDecision {
        CandidateDecision {
            disposition,
            reason: if disposition == Disposition::Keep {
                DecisionReason::PrimaryImprovement
            } else {
                DecisionReason::PrimaryRegression
            },
        }
    }

    fn entry(sequence: u64, event: JournalEvent) -> JournalEntry {
        JournalEntry {
            sequence,
            run_id: "run-1".into(),
            event,
        }
    }

    fn started() -> Vec<JournalEntry> {
        vec![entry(
            0,
            JournalEvent::RunStarted {
                base_commit: "abc".into(),
                frozen_identity: "frozen".into(),
            },
        )]
    }

    fn based() -> Vec<JournalEntry> {
        let mut entries = started();
        entries.push(entry(
            1,
            JournalEvent::BaselineCaptured {
                snapshot: snapshot(10.0),
            },
        ));
        entries
    }

    fn prepared() -> Vec<JournalEntry> {
        let mut entries = based();
        entries.push(entry(
            2,
            JournalEvent::CandidatePrepared {
                index: 1,
                parent_commit: "abc".into(),
                worktree_id: "candidate-1".into(),
            },
        ));
        entries
    }

    fn decided(disposition: Disposition) -> Vec<JournalEntry> {
        let mut entries = prepared();
        entries.push(entry(
            3,
            JournalEvent::CandidateDecisionRecorded {
                index: 1,
                candidate_commit: "def".into(),
                changed_paths: Vec::new(),
                snapshot: snapshot(11.0),
                decision: decision(disposition),
            },
        ));
        entries
    }

    #[test]
    fn partial_evaluator_capture_keeps_exact_recovery_action_and_commit() {
        let mut entries = prepared();
        entries.push(entry(
            3,
            JournalEvent::CandidateEvaluatorCaptured {
                index: 1,
                evaluator_id: "first".into(),
                evaluated_commit: "def".into(),
                output_json: "{}".into(),
            },
        ));
        assert_eq!(
            replay_journal(&entries)
                .expect("partial replay")
                .recovery_action(),
            RecoveryAction::EvaluateCandidate {
                index: 1,
                worktree_id: "candidate-1".into()
            }
        );
        entries.push(entry(
            4,
            JournalEvent::CandidateDecisionRecorded {
                index: 1,
                candidate_commit: "other".into(),
                changed_paths: Vec::new(),
                snapshot: snapshot(11.0),
                decision: decision(Disposition::Keep),
            },
        ));
        assert!(matches!(
            replay_journal(&entries),
            Err(JournalError::EvaluatorCommitMismatch)
        ));
    }

    #[test]
    fn duplicate_or_out_of_order_evaluator_capture_fails_closed() {
        let captured = JournalEvent::CandidateEvaluatorCaptured {
            index: 1,
            evaluator_id: "first".into(),
            evaluated_commit: "def".into(),
            output_json: "{}".into(),
        };
        let mut entries = prepared();
        entries.push(entry(3, captured.clone()));
        entries.push(entry(4, captured));
        assert!(matches!(
            replay_journal(&entries),
            Err(JournalError::DuplicateEvaluator(_))
        ));

        let mut wrong_stage = decided(Disposition::Keep);
        wrong_stage.push(entry(
            4,
            JournalEvent::CandidateEvaluatorCaptured {
                index: 1,
                evaluator_id: "late".into(),
                evaluated_commit: "def".into(),
                output_json: "{}".into(),
            },
        ));
        assert!(matches!(
            replay_journal(&wrong_stage),
            Err(JournalError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn each_valid_prefix_has_exact_recovery_action() {
        assert_eq!(
            replay_journal(&[])
                .expect("empty journal")
                .recovery_action(),
            RecoveryAction::StartRun
        );
        assert_eq!(
            replay_journal(&started())
                .expect("started journal")
                .recovery_action(),
            RecoveryAction::CaptureBaseline
        );
        assert_eq!(
            replay_journal(&based())
                .expect("baseline journal")
                .recovery_action(),
            RecoveryAction::PrepareCandidate {
                index: 1,
                parent_commit: "abc".into()
            }
        );
        assert_eq!(
            replay_journal(&prepared())
                .expect("prepared journal")
                .recovery_action(),
            RecoveryAction::EvaluateCandidate {
                index: 1,
                worktree_id: "candidate-1".into()
            }
        );
        assert_eq!(
            replay_journal(&decided(Disposition::Keep))
                .expect("decided journal")
                .recovery_action(),
            RecoveryAction::FinalizeCandidate {
                index: 1,
                worktree_id: "candidate-1".into(),
                candidate_commit: "def".into(),
                decision: decision(Disposition::Keep)
            }
        );
    }

    #[test]
    fn finalized_keep_advances_commit_and_candidate() {
        let mut entries = decided(Disposition::Keep);
        entries.push(entry(
            4,
            JournalEvent::CandidateFinalized {
                index: 1,
                outcome: CandidateFinalization::Kept {
                    commit: "def".into(),
                },
            },
        ));
        let ReplayState::Run(view) = replay_journal(&entries).expect("valid journal") else {
            panic!("expected run")
        };
        assert_eq!(view.current_commit(), "def");
        assert_eq!(view.completed_candidates(), 1);
        assert_eq!(
            view.recovery_action(),
            &RecoveryAction::PrepareCandidate {
                index: 2,
                parent_commit: "def".into()
            }
        );
    }

    #[test]
    fn finalized_discard_preserves_commit() {
        let mut entries = decided(Disposition::Discard);
        entries.push(entry(
            4,
            JournalEvent::CandidateFinalized {
                index: 1,
                outcome: CandidateFinalization::Discarded,
            },
        ));
        let ReplayState::Run(view) = replay_journal(&entries).expect("valid journal") else {
            panic!("expected run")
        };
        assert_eq!(view.current_commit(), "abc");
    }

    #[test]
    fn stopped_run_is_finished_and_rejects_more_events() {
        let mut entries = based();
        entries.push(entry(
            2,
            JournalEvent::RunStopped {
                reason: "budget_exhausted".into(),
            },
        ));
        assert_eq!(
            replay_journal(&entries)
                .expect("stopped journal")
                .recovery_action(),
            RecoveryAction::Finished
        );
        entries.push(entry(
            3,
            JournalEvent::RunStopped {
                reason: "again".into(),
            },
        ));
        assert!(matches!(
            replay_journal(&entries),
            Err(JournalError::InvalidTransition {
                state: "stopped",
                ..
            })
        ));
    }

    #[test]
    fn gaps_and_cross_run_entries_fail() {
        let mut gaps = started();
        gaps.push(entry(
            2,
            JournalEvent::BaselineCaptured {
                snapshot: snapshot(10.0),
            },
        ));
        assert!(matches!(
            replay_journal(&gaps),
            Err(JournalError::Sequence { .. })
        ));

        let mut crossed = based();
        crossed[1].run_id = "other".into();
        assert!(matches!(
            replay_journal(&crossed),
            Err(JournalError::RunIdMismatch { .. })
        ));
    }

    #[test]
    fn candidate_order_and_parent_are_enforced() {
        let mut wrong_index = based();
        wrong_index.push(entry(
            2,
            JournalEvent::CandidatePrepared {
                index: 2,
                parent_commit: "abc".into(),
                worktree_id: "candidate-2".into(),
            },
        ));
        assert!(matches!(
            replay_journal(&wrong_index),
            Err(JournalError::CandidateIndex { .. })
        ));

        let mut wrong_parent = based();
        wrong_parent.push(entry(
            2,
            JournalEvent::CandidatePrepared {
                index: 1,
                parent_commit: "wrong".into(),
                worktree_id: "candidate-1".into(),
            },
        ));
        assert!(matches!(
            replay_journal(&wrong_parent),
            Err(JournalError::ParentCommitMismatch { .. })
        ));
    }

    #[test]
    fn duplicate_or_out_of_order_events_fail() {
        let mut duplicate_start = started();
        duplicate_start.push(entry(
            1,
            JournalEvent::RunStarted {
                base_commit: "abc".into(),
                frozen_identity: "frozen".into(),
            },
        ));
        assert_eq!(
            replay_journal(&duplicate_start),
            Err(JournalError::DuplicateRunStart)
        );

        let decision_without_candidate = vec![
            started()[0].clone(),
            entry(
                1,
                JournalEvent::CandidateDecisionRecorded {
                    index: 1,
                    candidate_commit: "def".into(),
                    changed_paths: Vec::new(),
                    snapshot: snapshot(11.0),
                    decision: decision(Disposition::Keep),
                },
            ),
        ];
        assert!(matches!(
            replay_journal(&decision_without_candidate),
            Err(JournalError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn finalization_must_match_recorded_decision() {
        let mut entries = decided(Disposition::Keep);
        entries.push(entry(
            4,
            JournalEvent::CandidateFinalized {
                index: 1,
                outcome: CandidateFinalization::Discarded,
            },
        ));
        assert_eq!(
            replay_journal(&entries),
            Err(JournalError::FinalizationMismatch)
        );

        let mut wrong_commit = decided(Disposition::Keep);
        wrong_commit.push(entry(
            4,
            JournalEvent::CandidateFinalized {
                index: 1,
                outcome: CandidateFinalization::Kept {
                    commit: "other".into(),
                },
            },
        ));
        assert_eq!(
            replay_journal(&wrong_commit),
            Err(JournalError::FinalizationMismatch)
        );
    }

    #[test]
    fn blank_identity_fields_fail() {
        let mut entries = started();
        entries[0].run_id = " ".into();
        assert_eq!(replay_journal(&entries), Err(JournalError::Blank("run_id")));

        let mut entries = started();
        if let JournalEvent::RunStarted {
            frozen_identity, ..
        } = &mut entries[0].event
        {
            frozen_identity.clear();
        }
        assert_eq!(
            replay_journal(&entries),
            Err(JournalError::Blank("frozen_identity"))
        );
    }

    #[test]
    fn baseline_failure_is_typed_terminal_and_never_a_snapshot() {
        let mut entries = started();
        let failure = EvaluatorFailure {
            class: FailureClass::Timeout,
            detail: "declared evaluator timed out".into(),
        };
        entries.push(entry(
            1,
            JournalEvent::BaselineFailed {
                evaluator_id: "tests".into(),
                failure: failure.clone(),
            },
        ));
        let ReplayState::Run(view) = replay_journal(&entries).expect("typed failure journal")
        else {
            panic!("run expected")
        };
        assert!(view.baseline().is_none());
        assert_eq!(view.baseline_failure(), Some(&("tests".into(), failure)));
        assert_eq!(view.recovery_action(), &RecoveryAction::Finished);
        entries.push(entry(
            2,
            JournalEvent::BaselineCaptured {
                snapshot: snapshot(1.0),
            },
        ));
        assert!(matches!(
            replay_journal(&entries),
            Err(JournalError::InvalidTransition { .. })
        ));
    }
}
