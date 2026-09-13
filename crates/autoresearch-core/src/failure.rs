//! Evaluator failure identity shared by journal and evaluator SDK.

use serde::{Deserialize, Serialize};

/// Failure class distinct from a legitimate failed hard gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Evaluator explicitly reported inability to evaluate.
    Reported,
    /// Executable could not start.
    Spawn,
    /// Executable exited unsuccessfully.
    NonZeroExit,
    /// Invocation exceeded its deadline.
    Timeout,
    /// Invocation was cancelled.
    Cancelled,
    /// Captured output exceeded configured limit.
    OutputLimit,
    /// Process response violated wire protocol.
    Protocol,
    /// Output did not match frozen declaration or provenance.
    Validation,
}

/// Typed evaluator failure without fabricated metrics or gate passes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorFailure {
    /// Stable class for run stop policy and reports.
    pub class: FailureClass,
    /// Bounded, redacted diagnostic detail.
    pub detail: String,
}
