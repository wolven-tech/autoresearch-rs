//! Domain types and deterministic decision policy for autoresearch.
//!
//! Core stays free of process, filesystem, Git, and network concerns. It owns
//! only validated experiment measurements and candidate-selection semantics.

mod decision;
mod journal;
mod metric;

pub use decision::{
    CandidateDecision, Complexity, DecisionError, DecisionReason, Disposition, EvaluationSnapshot,
    TieBreaker, select_candidate,
};
pub use journal::{
    CandidateFinalization, JournalEntry, JournalError, JournalEvent, RecoveryAction, ReplayState,
    RunView, replay_journal,
};
pub use metric::{
    FiniteValue, GateOutcome, Measurement, MetricDirection, MetricError, MetricKind,
    NumericMetricKind,
};
