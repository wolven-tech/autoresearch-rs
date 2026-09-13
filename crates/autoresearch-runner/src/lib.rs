//! Provider-neutral orchestration for frozen autoresearch runs.

mod baseline;
mod command;
mod mutation;

pub use baseline::{
    BaselineCapture, BaselineExecutor, RunnerError, SubprocessBaselineExecutor,
    capture_existing_baseline,
};
pub use command::{CommandMutationAdapter, MutationCommandError, MutationCommandReport};
pub use mutation::{
    FrozenRubric, MutationError, MutationRequest, PriorDecision, submit_manual_candidate,
};
