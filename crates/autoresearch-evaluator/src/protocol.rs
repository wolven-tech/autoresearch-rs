//! Strict one-request/one-response JSON Lines evaluator process protocol.

use crate::{EvaluationContext, EvaluatorFailure, EvaluatorOutput};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use thiserror::Error;

/// Wire protocol version supported by this SDK.
pub const PROTOCOL_VERSION: u32 = 1;

/// One request written to evaluator stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRequest {
    /// Exact wire protocol version.
    pub protocol_version: u32,
    /// Frozen manifest evaluator ID.
    pub evaluator_id: String,
    /// Stable run identifier.
    pub run_id: String,
    /// Full baseline Git object ID.
    pub baseline_commit: String,
    /// Full evaluated Git object ID.
    pub evaluated_commit: String,
    /// Canonical isolated candidate worktree.
    pub candidate_worktree: PathBuf,
    /// Changed repository-relative paths in declared order.
    pub changed_paths: Vec<String>,
    /// Only explicitly declared environment values.
    pub declared_environment: BTreeMap<String, String>,
    /// Canonical run-owned artifact directory.
    pub artifact_directory: PathBuf,
    /// Stable cancellation identifier.
    pub cancellation_id: String,
}

impl ProtocolRequest {
    /// Copies a validated context into wire request for one evaluator ID.
    ///
    /// # Errors
    ///
    /// Rejects a blank evaluator ID.
    pub fn from_context(
        context: &EvaluationContext,
        evaluator_id: impl Into<String>,
    ) -> Result<Self, ProtocolError> {
        let evaluator_id = evaluator_id.into();
        if evaluator_id.trim().is_empty() {
            return Err(ProtocolError::InvalidEvaluatorId);
        }
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            evaluator_id,
            run_id: context.run_id().to_string(),
            baseline_commit: context.baseline_commit().to_string(),
            evaluated_commit: context.evaluated_commit().to_string(),
            candidate_worktree: context.candidate_worktree().to_path_buf(),
            changed_paths: context
                .changed_paths()
                .iter()
                .map(|path| path.as_str().to_owned())
                .collect(),
            declared_environment: context.declared_environment().clone(),
            artifact_directory: context.artifact_directory().to_path_buf(),
            cancellation_id: context.cancellation_id().to_string(),
        })
    }
}

/// One response read from evaluator stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolResponse {
    /// Exact wire protocol version.
    pub protocol_version: u32,
    /// Either comparable output candidate or explicit evaluator failure.
    pub result: ProtocolResult,
}

/// Mutually exclusive success and evaluator-reported failure shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProtocolResult {
    /// Candidate output requiring shared validation before use.
    Success {
        /// Untrusted typed evaluator output.
        output: EvaluatorOutput,
    },
    /// Evaluator could not produce comparable output.
    Failure {
        /// Explicit failure class and diagnostic.
        failure: EvaluatorFailure,
    },
}

/// Wire framing, schema, or output-write error.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Record did not end with one newline.
    #[error("JSONL record must end with a newline")]
    MissingNewline,
    /// Stdout contained more than one record or diagnostic text.
    #[error("expected exactly one JSONL record")]
    RecordCount,
    /// Bytes were not valid UTF-8.
    #[error("JSONL record must be UTF-8")]
    NonUtf8,
    /// Parsed JSON did not match strict schema.
    #[error("invalid evaluator JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Request/response version is unknown to this SDK.
    #[error("unsupported evaluator protocol version {0}")]
    UnsupportedVersion(u32),
    /// Evaluator ID was blank.
    #[error("evaluator ID cannot be blank")]
    InvalidEvaluatorId,
    /// Request could not be written to stdin.
    #[error("could not write evaluator request: {0}")]
    Write(#[from] std::io::Error),
}

/// Encodes exactly one newline-terminated request record.
///
/// # Errors
///
/// Rejects unknown version or JSON serialization failure.
pub fn encode_request(request: &ProtocolRequest) -> Result<Vec<u8>, ProtocolError> {
    check_version(request.protocol_version)?;
    if request.evaluator_id.trim().is_empty() {
        return Err(ProtocolError::InvalidEvaluatorId);
    }
    encode_line(request)
}

/// Writes one request record to a process stdin stream.
///
/// # Errors
///
/// Returns framing/serialization or I/O error.
pub fn write_request(
    stdin: &mut impl Write,
    request: &ProtocolRequest,
) -> Result<(), ProtocolError> {
    stdin.write_all(&encode_request(request)?)?;
    Ok(())
}

/// Decodes exactly one newline-terminated request record.
///
/// # Errors
///
/// Rejects invalid framing, UTF-8, schema, evaluator ID, or version.
pub fn decode_request(bytes: &[u8]) -> Result<ProtocolRequest, ProtocolError> {
    let request: ProtocolRequest = decode_line(bytes)?;
    check_version(request.protocol_version)?;
    if request.evaluator_id.trim().is_empty() {
        return Err(ProtocolError::InvalidEvaluatorId);
    }
    Ok(request)
}

/// Encodes exactly one newline-terminated response record.
///
/// # Errors
///
/// Rejects unknown version or JSON serialization failure.
pub fn encode_response(response: &ProtocolResponse) -> Result<Vec<u8>, ProtocolError> {
    check_version(response.protocol_version)?;
    encode_line(response)
}

/// Decodes exactly one newline-terminated response from stdout.
///
/// # Errors
///
/// Rejects invalid framing, UTF-8, schema, or version. Diagnostic text must
/// be written to stderr; stdout is reserved for this one response.
pub fn decode_response(bytes: &[u8]) -> Result<ProtocolResponse, ProtocolError> {
    let response: ProtocolResponse = decode_line(bytes)?;
    check_version(response.protocol_version)?;
    Ok(response)
}

fn check_version(version: u32) -> Result<(), ProtocolError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::UnsupportedVersion(version))
    }
}

fn encode_line(value: &impl Serialize) -> Result<Vec<u8>, ProtocolError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn decode_line<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ProtocolError> {
    if bytes.last() != Some(&b'\n') {
        return Err(ProtocolError::MissingNewline);
    }
    let body = &bytes[..bytes.len() - 1];
    if body.contains(&b'\n') {
        return Err(ProtocolError::RecordCount);
    }
    let source = std::str::from_utf8(body).map_err(|_| ProtocolError::NonUtf8)?;
    Ok(serde_json::from_str(source)?)
}
