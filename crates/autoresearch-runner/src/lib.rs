//! Provider-neutral orchestration for frozen autoresearch runs.

mod baseline;

pub use baseline::{
    BaselineCapture, BaselineExecutor, RunnerError, SubprocessBaselineExecutor,
    capture_existing_baseline,
};
