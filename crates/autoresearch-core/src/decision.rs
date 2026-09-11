//! Lexicographic candidate selection.

use crate::{Measurement, MetricDirection, MetricKind};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

/// Candidate properties used after equal primary objective.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Complexity {
    /// Added and removed lines in candidate diff.
    pub changed_lines: u64,
    /// Number of newly introduced dependencies.
    pub dependency_delta: u32,
    /// Candidate evaluation runtime in milliseconds.
    pub runtime_ms: u64,
}

/// Complete comparable evaluator output for one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSnapshot {
    /// Typed evaluator measurements.
    pub measurements: Vec<Measurement>,
    /// Frozen tie-breaker inputs.
    pub complexity: Complexity,
}

/// Final candidate disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Candidate becomes current best.
    Keep,
    /// Candidate is recorded but does not advance current best.
    Discard,
}

/// Complexity field that resolved equal objective values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TieBreaker {
    /// Smaller source diff won.
    ChangedLines,
    /// Fewer new dependencies won.
    DependencyDelta,
    /// Lower evaluation runtime won.
    Runtime,
}

/// Machine-readable reason for a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DecisionReason {
    /// Candidate failed one or more mandatory gates.
    FailedHardGates {
        /// Stable names of failed gates, in evaluator order.
        names: Vec<String>,
    },
    /// Candidate improved frozen primary objective.
    PrimaryImprovement,
    /// Candidate regressed frozen primary objective.
    PrimaryRegression,
    /// Equal objective resolved through approved complexity order.
    TieBreaker {
        /// First unequal complexity field.
        field: TieBreaker,
    },
    /// Candidate and baseline were equal across every selection input.
    NoImprovement,
}

/// Deterministic selection result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateDecision {
    /// Whether candidate advances.
    pub disposition: Disposition,
    /// Exact policy branch that produced disposition.
    pub reason: DecisionReason,
}

/// Invalid evaluation data that prevents a trustworthy decision.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecisionError {
    /// Measurement names must be unique inside each snapshot.
    #[error("duplicate measurement `{name}` in {snapshot} snapshot")]
    DuplicateMeasurement {
        /// Baseline or candidate.
        snapshot: &'static str,
        /// Duplicated stable name.
        name: String,
    },
    /// Baseline must be valid before candidate comparison.
    #[error("baseline failed hard gate `{0}`")]
    InvalidBaseline(String),
    /// Frozen objective is absent.
    #[error("missing objective `{name}` in {snapshot} snapshot")]
    MissingObjective {
        /// Baseline or candidate.
        snapshot: &'static str,
        /// Frozen objective name.
        name: String,
    },
    /// Frozen objective name points to a non-objective measurement.
    #[error("measurement `{name}` in {snapshot} snapshot is not an objective")]
    WrongObjectiveKind {
        /// Baseline or candidate.
        snapshot: &'static str,
        /// Frozen objective name.
        name: String,
    },
    /// Candidate changed objective direction after baseline.
    #[error("objective `{name}` direction differs between baseline and candidate")]
    ObjectiveDirectionMismatch {
        /// Frozen objective name.
        name: String,
    },
}

/// Applies frozen lexicographic policy to one candidate.
///
/// Diagnostic and market-evidence measurements are deliberately ignored.
/// Secondary metrics cannot rescue primary regression.
///
/// # Errors
///
/// Returns [`DecisionError`] for duplicate names, invalid baseline gates,
/// missing objectives, wrong objective kinds, or changed direction.
pub fn select_candidate(
    baseline: &EvaluationSnapshot,
    candidate: &EvaluationSnapshot,
    objective_name: &str,
) -> Result<CandidateDecision, DecisionError> {
    validate_snapshot(baseline, "baseline")?;
    validate_snapshot(candidate, "candidate")?;

    if let Some(name) = first_failed_gate(baseline) {
        return Err(DecisionError::InvalidBaseline(name.to_owned()));
    }

    let failed_gates = failed_gates(candidate);
    if !failed_gates.is_empty() {
        return Ok(CandidateDecision {
            disposition: Disposition::Discard,
            reason: DecisionReason::FailedHardGates {
                names: failed_gates,
            },
        });
    }

    let (baseline_direction, baseline_value) = objective(baseline, "baseline", objective_name)?;
    let (candidate_direction, candidate_value) = objective(candidate, "candidate", objective_name)?;
    if baseline_direction != candidate_direction {
        return Err(DecisionError::ObjectiveDirectionMismatch {
            name: objective_name.to_owned(),
        });
    }

    let ordering = candidate_value.cmp(&baseline_value);
    let improves = match baseline_direction {
        MetricDirection::Maximize => ordering.is_gt(),
        MetricDirection::Minimize => ordering.is_lt(),
    };
    let regresses = match baseline_direction {
        MetricDirection::Maximize => ordering.is_lt(),
        MetricDirection::Minimize => ordering.is_gt(),
    };

    if improves {
        return Ok(CandidateDecision {
            disposition: Disposition::Keep,
            reason: DecisionReason::PrimaryImprovement,
        });
    }
    if regresses {
        return Ok(CandidateDecision {
            disposition: Disposition::Discard,
            reason: DecisionReason::PrimaryRegression,
        });
    }

    Ok(compare_complexity(
        baseline.complexity,
        candidate.complexity,
    ))
}

fn validate_snapshot(
    snapshot: &EvaluationSnapshot,
    snapshot_name: &'static str,
) -> Result<(), DecisionError> {
    let mut names = HashSet::with_capacity(snapshot.measurements.len());
    for measurement in &snapshot.measurements {
        if !names.insert(measurement.name()) {
            return Err(DecisionError::DuplicateMeasurement {
                snapshot: snapshot_name,
                name: measurement.name().to_owned(),
            });
        }
    }
    Ok(())
}

fn first_failed_gate(snapshot: &EvaluationSnapshot) -> Option<&str> {
    snapshot
        .measurements
        .iter()
        .find_map(|measurement| match measurement {
            Measurement::HardGate { name, outcome } if !outcome.passed() => Some(name.as_str()),
            _ => None,
        })
}

fn failed_gates(snapshot: &EvaluationSnapshot) -> Vec<String> {
    snapshot
        .measurements
        .iter()
        .filter_map(|measurement| match measurement {
            Measurement::HardGate { name, outcome } if !outcome.passed() => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn objective(
    snapshot: &EvaluationSnapshot,
    snapshot_name: &'static str,
    objective_name: &str,
) -> Result<(MetricDirection, crate::FiniteValue), DecisionError> {
    let measurement = snapshot
        .measurements
        .iter()
        .find(|measurement| measurement.name() == objective_name)
        .ok_or_else(|| DecisionError::MissingObjective {
            snapshot: snapshot_name,
            name: objective_name.to_owned(),
        })?;

    match measurement {
        Measurement::Numeric {
            metric_kind,
            direction,
            value,
            ..
        } if MetricKind::from(*metric_kind) == MetricKind::Objective => Ok((*direction, *value)),
        _ => Err(DecisionError::WrongObjectiveKind {
            snapshot: snapshot_name,
            name: objective_name.to_owned(),
        }),
    }
}

fn compare_complexity(baseline: Complexity, candidate: Complexity) -> CandidateDecision {
    for (field, candidate_value, baseline_value) in [
        (
            TieBreaker::ChangedLines,
            candidate.changed_lines,
            baseline.changed_lines,
        ),
        (
            TieBreaker::DependencyDelta,
            u64::from(candidate.dependency_delta),
            u64::from(baseline.dependency_delta),
        ),
        (
            TieBreaker::Runtime,
            candidate.runtime_ms,
            baseline.runtime_ms,
        ),
    ] {
        if candidate_value < baseline_value {
            return CandidateDecision {
                disposition: Disposition::Keep,
                reason: DecisionReason::TieBreaker { field },
            };
        }
        if candidate_value > baseline_value {
            return CandidateDecision {
                disposition: Disposition::Discard,
                reason: DecisionReason::TieBreaker { field },
            };
        }
    }

    CandidateDecision {
        disposition: Disposition::Discard,
        reason: DecisionReason::NoImprovement,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Measurement, MetricDirection, NumericMetricKind};

    fn objective_measurement(value: f64, direction: MetricDirection) -> Measurement {
        Measurement::numeric("score", NumericMetricKind::Objective, direction, value)
            .expect("valid fixture")
    }

    fn snapshot(
        value: f64,
        direction: MetricDirection,
        complexity: Complexity,
    ) -> EvaluationSnapshot {
        EvaluationSnapshot {
            measurements: vec![
                Measurement::hard_gate("tests", true, None).expect("valid fixture"),
                objective_measurement(value, direction),
            ],
            complexity,
        }
    }

    #[test]
    fn failed_candidate_gate_discards_before_objective() {
        let baseline = snapshot(10.0, MetricDirection::Maximize, Complexity::default());
        let candidate = EvaluationSnapshot {
            measurements: vec![
                Measurement::hard_gate("tests", false, Some("one failure".into()))
                    .expect("valid fixture"),
                objective_measurement(99.0, MetricDirection::Maximize),
            ],
            complexity: Complexity::default(),
        };

        let decision = select_candidate(&baseline, &candidate, "score").expect("valid comparison");
        assert_eq!(decision.disposition, Disposition::Discard);
        assert_eq!(
            decision.reason,
            DecisionReason::FailedHardGates {
                names: vec!["tests".into()]
            }
        );
    }

    #[test]
    fn improving_primary_keeps_for_both_directions() {
        for (direction, baseline_value, candidate_value) in [
            (MetricDirection::Maximize, 10.0, 11.0),
            (MetricDirection::Minimize, 10.0, 9.0),
        ] {
            let baseline = snapshot(baseline_value, direction, Complexity::default());
            let candidate = snapshot(candidate_value, direction, Complexity::default());
            let decision =
                select_candidate(&baseline, &candidate, "score").expect("valid comparison");
            assert_eq!(decision.disposition, Disposition::Keep);
            assert_eq!(decision.reason, DecisionReason::PrimaryImprovement);
        }
    }

    #[test]
    fn regressing_primary_discards_despite_better_diagnostic_and_market_evidence() {
        let baseline = snapshot(10.0, MetricDirection::Maximize, Complexity::default());
        let mut candidate = snapshot(9.0, MetricDirection::Maximize, Complexity::default());
        candidate.measurements.extend([
            Measurement::numeric(
                "lighthouse",
                NumericMetricKind::Diagnostic,
                MetricDirection::Maximize,
                100.0,
            )
            .expect("valid fixture"),
            Measurement::numeric(
                "clicks",
                NumericMetricKind::MarketEvidence,
                MetricDirection::Maximize,
                1_000_000.0,
            )
            .expect("valid fixture"),
        ]);

        let decision = select_candidate(&baseline, &candidate, "score").expect("valid comparison");
        assert_eq!(decision.disposition, Disposition::Discard);
        assert_eq!(decision.reason, DecisionReason::PrimaryRegression);
    }

    #[test]
    fn equal_primary_uses_lexicographic_complexity() {
        let baseline = snapshot(
            10.0,
            MetricDirection::Maximize,
            Complexity {
                changed_lines: 20,
                dependency_delta: 1,
                runtime_ms: 100,
            },
        );
        let candidate = snapshot(
            10.0,
            MetricDirection::Maximize,
            Complexity {
                changed_lines: 20,
                dependency_delta: 0,
                runtime_ms: 1_000,
            },
        );

        let decision = select_candidate(&baseline, &candidate, "score").expect("valid comparison");
        assert_eq!(decision.disposition, Disposition::Keep);
        assert_eq!(
            decision.reason,
            DecisionReason::TieBreaker {
                field: TieBreaker::DependencyDelta
            }
        );
    }

    #[test]
    fn exact_tie_discards() {
        let baseline = snapshot(10.0, MetricDirection::Maximize, Complexity::default());
        let candidate = baseline.clone();
        let decision = select_candidate(&baseline, &candidate, "score").expect("valid comparison");
        assert_eq!(decision.disposition, Disposition::Discard);
        assert_eq!(decision.reason, DecisionReason::NoImprovement);
    }

    #[test]
    fn malformed_snapshots_fail_closed() {
        let baseline = snapshot(10.0, MetricDirection::Maximize, Complexity::default());
        let mut duplicate = baseline.clone();
        duplicate
            .measurements
            .push(objective_measurement(11.0, MetricDirection::Maximize));
        assert!(matches!(
            select_candidate(&baseline, &duplicate, "score"),
            Err(DecisionError::DuplicateMeasurement { .. })
        ));

        let missing = EvaluationSnapshot {
            measurements: vec![Measurement::hard_gate("tests", true, None).expect("valid fixture")],
            complexity: Complexity::default(),
        };
        assert!(matches!(
            select_candidate(&baseline, &missing, "score"),
            Err(DecisionError::MissingObjective { .. })
        ));

        let wrong_direction = snapshot(11.0, MetricDirection::Minimize, Complexity::default());
        assert!(matches!(
            select_candidate(&baseline, &wrong_direction, "score"),
            Err(DecisionError::ObjectiveDirectionMismatch { .. })
        ));
    }

    #[test]
    fn failed_baseline_is_rejected() {
        let mut baseline = snapshot(10.0, MetricDirection::Maximize, Complexity::default());
        baseline.measurements[0] =
            Measurement::hard_gate("tests", false, None).expect("valid fixture");
        let candidate = snapshot(11.0, MetricDirection::Maximize, Complexity::default());
        assert_eq!(
            select_candidate(&baseline, &candidate, "score"),
            Err(DecisionError::InvalidBaseline("tests".into()))
        );
    }
}
