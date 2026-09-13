//! Bounded JSONL evaluator process for isolated tiny Candle training.

use autoresearch_evaluator::{
    EvaluationContext, EvaluationContextSpec, EvaluatorFailure, FailureClass,
    NativeEvaluationError, PROTOCOL_VERSION, ProtocolRequest, ProtocolResponse, ProtocolResult,
    decode_request, encode_response, evaluate_native,
};
use autoresearch_training_candle::{TRAINING_EVALUATOR_ID, TrainingEvaluator};
use std::io::{Read, Write};

const MAX_REQUEST_BYTES: u64 = 1_048_576;

fn main() {
    if run().is_err() {
        eprintln!("training evaluator request rejected");
        std::process::exit(2);
    }
}

fn run() -> Result<(), ()> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input)
        .map_err(|_| ())?;
    if u64::try_from(input.len()).map_err(|_| ())? > MAX_REQUEST_BYTES {
        return Err(());
    }
    let request = decode_request(&input).map_err(|_| ())?;
    let result = evaluate_request(request);
    let bytes = encode_response(&ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result,
    })
    .map_err(|_| ())?;
    std::io::stdout().write_all(&bytes).map_err(|_| ())
}

fn evaluate_request(request: ProtocolRequest) -> ProtocolResult {
    if request.evaluator_id != TRAINING_EVALUATOR_ID {
        return ProtocolResult::Failure {
            failure: failure(FailureClass::Validation, "training evaluator ID mismatch"),
        };
    }
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: request.run_id,
        baseline_commit: request.baseline_commit,
        evaluated_commit: request.evaluated_commit,
        candidate_worktree: request.candidate_worktree,
        changed_paths: request.changed_paths,
        declared_environment: request.declared_environment,
        artifact_directory: request.artifact_directory,
        cancellation_id: request.cancellation_id,
    });
    let Ok(context) = context else {
        return ProtocolResult::Failure {
            failure: failure(FailureClass::Validation, "training context invalid"),
        };
    };
    match evaluate_native(&context, &TrainingEvaluator::default()) {
        Ok(output) => ProtocolResult::Success {
            output: output.into_output(),
        },
        Err(NativeEvaluationError::Evaluator(failure)) => ProtocolResult::Failure { failure },
        Err(NativeEvaluationError::Output(_)) => ProtocolResult::Failure {
            failure: failure(FailureClass::Validation, "training output invalid"),
        },
    }
}

fn failure(class: FailureClass, detail: &'static str) -> EvaluatorFailure {
    EvaluatorFailure {
        class,
        detail: detail.into(),
    }
}
