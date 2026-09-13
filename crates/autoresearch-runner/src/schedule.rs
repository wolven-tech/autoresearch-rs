//! One-at-a-time candidate scheduling under frozen run bounds.

use autoresearch_config::Budget;
use autoresearch_core::RepoPath;
use autoresearch_evaluator::CancellationToken;
use serde::Serialize;
use std::time::{Duration, Instant};
use thiserror::Error;

/// One bounded candidate attempt result. Invalid candidates consume failure
/// budget; they never become comparable evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAttempt {
    /// Candidate finished full mutation/evaluation/decision lifecycle.
    Completed,
    /// Candidate mutation failed containment or yielded no valid diff.
    Invalid,
    /// Declared evaluator failed and no comparable score was created.
    EvaluatorFailed,
    /// Operator explicitly requested stop while attempt was active.
    StopRequested,
}

/// Exact reason no further candidate may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Frozen candidate count exhausted.
    CandidateLimit,
    /// Frozen invalid/evaluator failure count exhausted.
    FailureLimit,
    /// Frozen wall-clock ceiling reached.
    WallClockLimit,
    /// Cancellation token requested stop.
    Cancelled,
    /// Operator explicitly stopped the run.
    OperatorRequested,
}

/// Number of serial attempts and why loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ScheduleReport {
    /// Number of candidate callbacks invoked.
    pub attempted: u32,
    /// Number of invalid mutation candidates.
    pub invalid: u32,
    /// Number of evaluator failures.
    pub evaluator_failures: u32,
    /// Deterministic stopping condition.
    pub stop_reason: StopReason,
}

/// Refusal before any candidate can start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScheduleError {
    /// Zero candidate, failure, or wall-clock budget is unsafe.
    #[error("run budget requires nonzero candidate, failure, and wall-clock limits")]
    InvalidBudget,
    /// No writable root was declared.
    #[error("no valid mutable path available for candidate")]
    NoMutablePath,
}

/// Runs one callback at a time and never schedules above frozen limits.
/// Callback receives one-based candidate index and remaining wall-clock cap.
/// It must obey remaining cap when launching its own bounded children; runner
/// checks clock again after return. This function never creates parallel work.
///
/// # Errors
///
/// Rejects zero budgets or empty mutable scope before invoking callback.
pub fn run_serial_candidates(
    budget: Budget,
    allowed_files: &[RepoPath],
    cancellation: &CancellationToken,
    mut attempt: impl FnMut(u32, Duration) -> CandidateAttempt,
) -> Result<ScheduleReport, ScheduleError> {
    if budget.max_candidates == 0 || budget.max_failures == 0 || budget.wall_clock_seconds == 0 {
        return Err(ScheduleError::InvalidBudget);
    }
    if allowed_files.is_empty() {
        return Err(ScheduleError::NoMutablePath);
    }
    let started = Instant::now();
    let ceiling = Duration::from_secs(budget.wall_clock_seconds);
    let mut report = ScheduleReport {
        attempted: 0,
        invalid: 0,
        evaluator_failures: 0,
        stop_reason: StopReason::CandidateLimit,
    };
    loop {
        if cancellation.is_cancelled() {
            report.stop_reason = StopReason::Cancelled;
            break;
        }
        let remaining = ceiling.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            report.stop_reason = StopReason::WallClockLimit;
            break;
        }
        if report.attempted >= budget.max_candidates {
            report.stop_reason = StopReason::CandidateLimit;
            break;
        }
        let index = report.attempted + 1;
        let outcome = attempt(index, remaining);
        report.attempted = index;
        match outcome {
            CandidateAttempt::Completed => {}
            CandidateAttempt::Invalid => report.invalid += 1,
            CandidateAttempt::EvaluatorFailed => report.evaluator_failures += 1,
            CandidateAttempt::StopRequested => {
                report.stop_reason = StopReason::OperatorRequested;
                break;
            }
        }
        if cancellation.is_cancelled() {
            report.stop_reason = StopReason::Cancelled;
            break;
        }
        if started.elapsed() >= ceiling {
            report.stop_reason = StopReason::WallClockLimit;
            break;
        }
        if report.invalid + report.evaluator_failures >= budget.max_failures {
            report.stop_reason = StopReason::FailureLimit;
            break;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(candidates: u32, failures: u32, seconds: u64) -> Budget {
        Budget {
            max_candidates: candidates,
            max_failures: failures,
            wall_clock_seconds: seconds,
        }
    }

    fn paths() -> Vec<RepoPath> {
        vec![RepoPath::new("web/src").expect("mutable root")]
    }

    #[test]
    fn serial_completed_candidates_stop_at_exact_count() {
        let mut indices = Vec::new();
        let report = run_serial_candidates(
            budget(3, 2, 10),
            &paths(),
            &CancellationToken::default(),
            |index, remaining| {
                assert!(!remaining.is_zero());
                indices.push(index);
                CandidateAttempt::Completed
            },
        )
        .expect("bounded run");
        assert_eq!(indices, [1, 2, 3]);
        assert_eq!(report.attempted, 3);
        assert_eq!(report.stop_reason, StopReason::CandidateLimit);
    }

    #[test]
    fn repeated_invalid_and_evaluator_failures_share_failure_ceiling() {
        let mut indices = Vec::new();
        let report = run_serial_candidates(
            budget(100, 3, 10),
            &paths(),
            &CancellationToken::default(),
            |index, _| {
                indices.push(index);
                if index == 2 {
                    CandidateAttempt::EvaluatorFailed
                } else {
                    CandidateAttempt::Invalid
                }
            },
        )
        .expect("bounded failures");
        assert_eq!(indices, [1, 2, 3]);
        assert_eq!(report.invalid, 2);
        assert_eq!(report.evaluator_failures, 1);
        assert_eq!(report.stop_reason, StopReason::FailureLimit);
    }

    #[test]
    fn cancellation_and_explicit_stop_never_start_next_candidate() {
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let cancelled = run_serial_candidates(budget(3, 2, 10), &paths(), &cancellation, |_, _| {
            panic!("cancelled run must not call candidate")
        })
        .expect("cancelled report");
        assert_eq!(cancelled.attempted, 0);
        assert_eq!(cancelled.stop_reason, StopReason::Cancelled);

        let stopped = run_serial_candidates(
            budget(3, 2, 10),
            &paths(),
            &CancellationToken::default(),
            |_, _| CandidateAttempt::StopRequested,
        )
        .expect("operator stop");
        assert_eq!(stopped.attempted, 1);
        assert_eq!(stopped.stop_reason, StopReason::OperatorRequested);
    }

    #[test]
    fn wall_clock_stops_after_active_bounded_callback_returns() {
        let report = run_serial_candidates(
            budget(3, 2, 1),
            &paths(),
            &CancellationToken::default(),
            |_, _| {
                std::thread::sleep(Duration::from_millis(1_100));
                CandidateAttempt::Completed
            },
        )
        .expect("clock-limited run");
        assert_eq!(report.attempted, 1);
        assert_eq!(report.stop_reason, StopReason::WallClockLimit);
    }

    #[test]
    fn zero_budget_and_no_mutable_root_refuse_before_callback() {
        for invalid in [budget(0, 1, 1), budget(1, 0, 1), budget(1, 1, 0)] {
            assert!(matches!(
                run_serial_candidates(invalid, &paths(), &CancellationToken::default(), |_, _| {
                    panic!("invalid budget must not call candidate")
                }),
                Err(ScheduleError::InvalidBudget)
            ));
        }
        assert!(matches!(
            run_serial_candidates(
                budget(1, 1, 1),
                &[],
                &CancellationToken::default(),
                |_, _| panic!("no scope must not call candidate")
            ),
            Err(ScheduleError::NoMutablePath)
        ));
    }
}
