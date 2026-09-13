//! Domain types and deterministic decision policy for autoresearch.
//!
//! Core stays free of process, filesystem I/O, Git commands, and network
//! concerns. It owns validated experiment values and selection semantics.

mod containment;
mod decision;
mod failure;
mod journal;
mod metric;
mod recovery;
mod repository;
mod workspace;

pub use containment::{
    CandidateCommit, CandidateCommitError, CandidateCommitter, ContainmentViolation,
    MutationBoundary, MutationBoundaryError, RepoPath, RepoPathError,
};
pub use decision::{
    CandidateDecision, Complexity, DecisionError, DecisionReason, Disposition, EvaluationSnapshot,
    TieBreaker, select_candidate,
};
pub use failure::{EvaluatorFailure, FailureClass};
pub use journal::{
    CandidateFinalization, JournalEntry, JournalError, JournalEvent, RecoveryAction, ReplayState,
    RunView, replay_journal,
};
pub use metric::{
    FiniteValue, GateOutcome, Measurement, MetricDirection, MetricError, MetricKind,
    NumericMetricKind,
};
pub use recovery::{
    CandidateRecovery, CandidateRecoveryManager, CandidateRecoveryRequest, RecoveredCandidateState,
    RecoveryValueError,
};
pub use repository::{CommitId, RepositoryInspector, RepositorySnapshot, RepositoryValueError};
pub use workspace::{
    CandidateWorkspace, CandidateWorkspaceManager, RunId, RunWorkspace, WorkspaceValueError,
};
