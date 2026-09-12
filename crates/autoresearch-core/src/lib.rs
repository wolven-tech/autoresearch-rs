//! Domain types and deterministic decision policy for autoresearch.
//!
//! Core stays free of process, filesystem I/O, Git commands, and network
//! concerns. It owns validated experiment values and selection semantics.

mod decision;
mod journal;
mod metric;
mod repository;
mod workspace;

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
pub use repository::{CommitId, RepositoryInspector, RepositorySnapshot, RepositoryValueError};
pub use workspace::{
    CandidateWorkspace, CandidateWorkspaceManager, RunId, RunWorkspace, WorkspaceValueError,
};
