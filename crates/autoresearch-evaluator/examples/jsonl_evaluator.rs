//! Minimal JSONL v1 subprocess evaluator: one request in, one response out.

use autoresearch_core::{Measurement, MetricDirection, NumericMetricKind};
use autoresearch_evaluator::{
    EvaluatorOutput, PROTOCOL_VERSION, ProtocolRequest, ProtocolResponse, ProtocolResult,
    decode_request, encode_response,
};
use std::error::Error;
use std::io::{Read, Write};

/// Handles exactly one protocol-v1 request record with no external service.
///
/// # Errors
///
/// Rejects malformed request framing or measurement construction failure.
pub fn handle(request_bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let request: ProtocolRequest = decode_request(request_bytes)?;
    let response = ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result: ProtocolResult::Success {
            output: EvaluatorOutput {
                evaluator_id: request.evaluator_id,
                run_id: request.run_id,
                baseline_commit: request.baseline_commit,
                evaluated_commit: request.evaluated_commit,
                measurements: vec![
                    Measurement::hard_gate("tests", true, None)?,
                    Measurement::numeric(
                        "score",
                        NumericMetricKind::Objective,
                        MetricDirection::Maximize,
                        1.25,
                    )?,
                ],
                observations: vec![],
                artifacts: vec![],
                warnings: vec![],
            },
        },
    };
    Ok(encode_response(&response)?)
}

fn main() {
    let mut request = Vec::new();
    let result = std::io::stdin()
        .read_to_end(&mut request)
        .map_err(|error| Box::new(error) as Box<dyn Error>)
        .and_then(|_| handle(&request))
        .and_then(|response| {
            std::io::stdout()
                .write_all(&response)
                .map_err(|error| Box::new(error) as Box<dyn Error>)
        });
    if result.is_err() {
        eprintln!("evaluator request invalid or response unavailable");
        std::process::exit(2);
    }
}
