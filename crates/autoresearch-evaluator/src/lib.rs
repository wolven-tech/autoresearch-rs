//! Typed evaluator boundary for native and subprocess adapters.
//!
//! Native implementations link statically. External executables use the
//! versioned process protocol; neither route may construct a comparable
//! snapshot before its output passes the shared validator.

mod cargo;
mod command;
mod context;
mod diff;
mod output;
mod process;
mod protocol;
mod snapshot;

pub use cargo::{CargoCheck, evaluate_cargo_check};
pub use command::evaluate_command_gate;
pub use context::{CancellationId, ContextError, EvaluationContext, EvaluationContextSpec};
pub use diff::{
    DependencyEvidence, DependencyManifest, DiffError, DiffEvidence, evaluate_commit_diff,
};
pub use output::{
    Artifact, EvaluatorFailure, EvaluatorOutput, FailureClass, NativeEvaluationError,
    NativeEvaluator, Observation, OutputError, ValidatedOutput, Warning, evaluate_native,
    validate_output,
};
pub use process::{
    CancellationToken, ProcessDiagnostics, ProcessEvaluation, ProcessFailure, ProcessLimitError,
    ProcessLimits, evaluate_subprocess,
};
pub use protocol::{
    PROTOCOL_VERSION, ProtocolError, ProtocolRequest, ProtocolResponse, ProtocolResult,
    decode_request, decode_response, encode_request, encode_response, write_request,
};
pub use snapshot::{ValidationError, build_snapshot};
