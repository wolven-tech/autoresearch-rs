//! Golden and rejection tests for the versioned evaluator JSONL protocol.

use autoresearch_core::{Measurement, MetricDirection, NumericMetricKind};
use autoresearch_evaluator::{
    EvaluatorFailure, EvaluatorOutput, FailureClass, PROTOCOL_VERSION, ProtocolError,
    ProtocolRequest, ProtocolResponse, ProtocolResult, decode_request, decode_response,
    encode_request, encode_response, write_request,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

const REQUEST_GOLDEN: &[u8] = include_bytes!("fixtures/request-v1.jsonl");
const SUCCESS_GOLDEN: &[u8] = include_bytes!("fixtures/response-success-v1.jsonl");
const FAILURE_GOLDEN: &[u8] = include_bytes!("fixtures/response-failure-v1.jsonl");
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn request() -> ProtocolRequest {
    ProtocolRequest {
        protocol_version: PROTOCOL_VERSION,
        evaluator_id: "fixture".into(),
        run_id: "run-1".into(),
        baseline_commit: COMMIT.into(),
        evaluated_commit: COMMIT.into(),
        candidate_worktree: PathBuf::from("/tmp/autoresearch/worktrees/run-1/candidate-000001"),
        changed_paths: vec!["src/lib.rs".into()],
        declared_environment: BTreeMap::from([("CARGO_TERM_COLOR".into(), "never".into())]),
        artifact_directory: PathBuf::from("/tmp/autoresearch/runs/run-1/artifacts"),
        cancellation_id: "cancel-1".into(),
    }
}

fn success() -> ProtocolResponse {
    ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result: ProtocolResult::Success {
            output: EvaluatorOutput {
                evaluator_id: "fixture".into(),
                run_id: "run-1".into(),
                baseline_commit: COMMIT.into(),
                evaluated_commit: COMMIT.into(),
                measurements: vec![
                    Measurement::hard_gate("tests", true, None).expect("valid gate"),
                    Measurement::numeric(
                        "score",
                        NumericMetricKind::Objective,
                        MetricDirection::Maximize,
                        1.25,
                    )
                    .expect("valid metric"),
                ],
                observations: vec![],
                artifacts: vec![],
                warnings: vec![],
            },
        },
    }
}

fn failure() -> ProtocolResponse {
    ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result: ProtocolResult::Failure {
            failure: EvaluatorFailure {
                class: FailureClass::Reported,
                detail: "fixture unavailable".into(),
            },
        },
    }
}

#[test]
fn request_golden_is_exactly_one_newline_terminated_record() {
    let bytes = encode_request(&request()).expect("encode request");
    assert_eq!(bytes, REQUEST_GOLDEN);
    assert_eq!(
        decode_request(REQUEST_GOLDEN).expect("decode request"),
        request()
    );
    let mut stdin = Vec::new();
    write_request(&mut stdin, &request()).expect("write request to stdin fixture");
    assert_eq!(stdin, REQUEST_GOLDEN);
    assert_eq!(stdin.last(), Some(&b'\n'));
    assert!(!stdin[..stdin.len() - 1].contains(&b'\n'));
}

#[test]
fn success_and_failure_response_goldens_round_trip() {
    assert_eq!(
        encode_response(&success()).expect("success encoding"),
        SUCCESS_GOLDEN
    );
    assert_eq!(
        decode_response(SUCCESS_GOLDEN).expect("success decoding"),
        success()
    );
    assert_eq!(
        encode_response(&failure()).expect("failure encoding"),
        FAILURE_GOLDEN
    );
    assert_eq!(
        decode_response(FAILURE_GOLDEN).expect("failure decoding"),
        failure()
    );
}

#[test]
fn rejects_old_and_unknown_protocol_versions() {
    for version in [0, 2, 999] {
        let request = String::from_utf8(REQUEST_GOLDEN.to_vec()).expect("UTF-8 fixture");
        let changed = request.replace(
            "\"protocol_version\":1",
            &format!("\"protocol_version\":{version}"),
        );
        assert!(matches!(
            decode_request(changed.as_bytes()),
            Err(ProtocolError::UnsupportedVersion(version_found)) if version_found == version
        ));
        let response = String::from_utf8(SUCCESS_GOLDEN.to_vec()).expect("UTF-8 fixture");
        let changed = response.replace(
            "\"protocol_version\":1",
            &format!("\"protocol_version\":{version}"),
        );
        assert!(matches!(
            decode_response(changed.as_bytes()),
            Err(ProtocolError::UnsupportedVersion(version_found)) if version_found == version
        ));
    }
}

#[test]
fn rejects_missing_unknown_and_malformed_fields() {
    let missing = String::from_utf8(REQUEST_GOLDEN.to_vec())
        .expect("UTF-8 fixture")
        .replace("\"run_id\":\"run-1\",", "");
    assert!(matches!(
        decode_request(missing.as_bytes()),
        Err(ProtocolError::Json(_))
    ));

    let unknown = String::from_utf8(REQUEST_GOLDEN.to_vec())
        .expect("UTF-8 fixture")
        .replace(
            "\"protocol_version\":1,",
            "\"protocol_version\":1,\"surprise\":true,",
        );
    assert!(matches!(
        decode_request(unknown.as_bytes()),
        Err(ProtocolError::Json(_))
    ));

    assert!(matches!(
        decode_response(b"{broken}\n"),
        Err(ProtocolError::Json(_))
    ));
    let response = String::from_utf8(SUCCESS_GOLDEN.to_vec()).expect("UTF-8 fixture");
    let unexpected = response.replace(
        "\"protocol_version\":1,",
        "\"protocol_version\":1,\"surprise\":true,",
    );
    assert!(matches!(
        decode_response(unexpected.as_bytes()),
        Err(ProtocolError::Json(_))
    ));
}

#[test]
fn rejects_extra_stdout_missing_newline_and_non_utf8() {
    let mut doubled = SUCCESS_GOLDEN.to_vec();
    doubled.extend_from_slice(SUCCESS_GOLDEN);
    assert!(matches!(
        decode_response(&doubled),
        Err(ProtocolError::RecordCount)
    ));

    let mut stdout_log = b"debugging\n".to_vec();
    stdout_log.extend_from_slice(SUCCESS_GOLDEN);
    assert!(matches!(
        decode_response(&stdout_log),
        Err(ProtocolError::RecordCount)
    ));

    assert!(matches!(
        decode_response(b"{}"),
        Err(ProtocolError::MissingNewline)
    ));
    let mut non_utf8 = SUCCESS_GOLDEN.to_vec();
    non_utf8.insert(1, 0xff);
    assert!(matches!(
        decode_response(&non_utf8),
        Err(ProtocolError::NonUtf8)
    ));
}
