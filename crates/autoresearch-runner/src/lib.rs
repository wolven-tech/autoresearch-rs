//! Provider-neutral orchestration for frozen autoresearch runs.

mod baseline;
mod candidate;
mod command;
mod environment;
mod mutation;
mod report_source;
mod resume;
mod run;
mod schedule;
mod status;
mod verify;

pub use baseline::{
    BaselineCapture, BaselineExecutor, RunnerError, SubprocessBaselineExecutor,
    capture_existing_baseline,
};
pub use candidate::{CandidateOutcome, evaluate_committed_candidate};
pub use command::{CommandMutationAdapter, MutationCommandError, MutationCommandReport};
pub use environment::EnvironmentRecord;
pub use mutation::{
    FrozenRubric, MutationError, MutationRequest, PriorDecision, submit_manual_candidate,
};
pub use report_source::{ReportSource, load_report_source};
pub use resume::{ResumeOutcome, resume_run};
pub use run::{RunMutationMode, RunStep, advance_run_once};
pub use schedule::{
    CandidateAttempt, ScheduleError, ScheduleReport, StopReason, run_serial_candidates,
};
pub use status::{RunStatus, inspect_run, stop_run};
pub use verify::{FreshEvaluation, VerificationEvidence, verify_kept_commit};
