//! Bounded child-process execution for versioned JSONL evaluators.

use crate::{
    EvaluationContext, EvaluatorFailure, FailureClass, ProtocolRequest, ProtocolResult,
    ValidatedOutput, decode_response, encode_request, validate_output,
};
use autoresearch_config::Evaluator;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_REQUEST_BYTES: usize = 1_048_576;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Cooperative cancellation shared with the runner.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Requests cancellation of current and future invocations using token.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Returns whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Invalid process output limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("stdout and stderr limits must be greater than zero")]
pub struct ProcessLimitError;

/// Maximum captured bytes per output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLimits {
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
}

impl ProcessLimits {
    /// Sets non-zero independent stdout/stderr byte caps.
    ///
    /// # Errors
    ///
    /// Rejects zero limits.
    pub fn new(
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<Self, ProcessLimitError> {
        if max_stdout_bytes == 0 || max_stderr_bytes == 0 {
            return Err(ProcessLimitError);
        }
        Ok(Self {
            max_stdout_bytes,
            max_stderr_bytes,
        })
    }
}

/// Output sizes and deliberately redacted stderr summary for reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessDiagnostics {
    /// Bytes received from stdout, capped by configured output limit.
    pub stdout_bytes: usize,
    /// Bytes received from stderr, capped by configured output limit.
    pub stderr_bytes: usize,
    /// Never contains raw evaluator stderr or ambient credentials.
    pub redacted_stderr: String,
}

impl ProcessDiagnostics {
    pub(crate) fn new(stdout_bytes: usize, stderr_bytes: usize) -> Self {
        Self {
            stdout_bytes,
            stderr_bytes,
            redacted_stderr: format!("[stderr redacted: {stderr_bytes} bytes captured]"),
        }
    }
}

/// Successful process output after shared structural validation.
#[derive(Debug)]
pub struct ProcessEvaluation {
    /// Validated output, not yet a complete manifest-matched snapshot.
    pub output: ValidatedOutput,
    /// Redacted process diagnostics.
    pub diagnostics: ProcessDiagnostics,
}

/// Process failure; never carries fabricated metrics or gate passes.
#[derive(Debug)]
pub struct ProcessFailure {
    /// Typed failure classification and redacted reason.
    pub failure: EvaluatorFailure,
    /// Redacted output summary.
    pub diagnostics: ProcessDiagnostics,
}

impl ProcessFailure {
    pub(crate) fn new(
        class: FailureClass,
        detail: impl Into<String>,
        stdout: usize,
        stderr: usize,
    ) -> Self {
        Self {
            failure: EvaluatorFailure {
                class,
                detail: detail.into(),
            },
            diagnostics: ProcessDiagnostics::new(stdout, stderr),
        }
    }
}

enum StreamEvent {
    Stdin(Result<(), ()>),
    Stdout(Result<Vec<u8>, usize>),
    Stderr(Result<Vec<u8>, usize>),
}

/// Captured process data before adapter-specific interpretation.
#[derive(Debug)]
pub(crate) struct ProcessCapture {
    pub(crate) status: ExitStatus,
    pub(crate) stdin_result: Result<(), ()>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// Executes declared evaluator command in candidate worktree with scrubbed environment.
///
/// Program and arguments come directly from frozen [`Evaluator`] command
/// fields; no shell interpolation occurs. Process-level network/filesystem
/// isolation and descendant-process cleanup are not guaranteed by this SDK.
///
/// # Errors
///
/// Returns explicit spawn, protocol, non-zero-exit, timeout, cancellation,
/// output-limit, evaluator-reported, or validation failure. Owned child is
/// terminated and reaped on timeout/cancellation/output overflow.
pub fn evaluate_subprocess(
    context: &EvaluationContext,
    evaluator: &Evaluator,
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessEvaluation, ProcessFailure> {
    let request = ProtocolRequest::from_context(context, evaluator.id())
        .map_err(|error| ProcessFailure::new(FailureClass::Protocol, error.to_string(), 0, 0))?;
    let request_bytes = encode_request(&request)
        .map_err(|error| ProcessFailure::new(FailureClass::Protocol, error.to_string(), 0, 0))?;
    let captured = execute_capture(
        context,
        evaluator,
        limits,
        cancellation,
        Some(request_bytes),
    )?;
    finish_process(context, evaluator, captured)
}

/// Runs frozen command with shared timeout, output, environment, and cancellation policy.
pub(crate) fn execute_capture(
    context: &EvaluationContext,
    evaluator: &Evaluator,
    limits: ProcessLimits,
    cancellation: &CancellationToken,
    request_bytes: Option<Vec<u8>>,
) -> Result<ProcessCapture, ProcessFailure> {
    if cancellation.is_cancelled() {
        return Err(ProcessFailure::new(
            FailureClass::Cancelled,
            "cancelled before process start",
            0,
            0,
        ));
    }
    if request_bytes
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAX_REQUEST_BYTES)
    {
        return Err(ProcessFailure::new(
            FailureClass::OutputLimit,
            "evaluator request exceeds byte limit",
            0,
            0,
        ));
    }

    let (mut child, receiver) = spawn_process(context, evaluator, limits, request_bytes)?;
    collect_process(evaluator, &mut child, &receiver, cancellation)
}

fn spawn_process(
    context: &EvaluationContext,
    evaluator: &Evaluator,
    limits: ProcessLimits,
    request_bytes: Option<Vec<u8>>,
) -> Result<(Child, mpsc::Receiver<StreamEvent>), ProcessFailure> {
    let command = evaluator.command();
    let stdin_mode = if request_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(command.program())
        .args(command.args())
        .current_dir(context.candidate_worktree())
        .env_clear()
        .envs(context.declared_environment())
        .stdin(stdin_mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ProcessFailure::new(
                FailureClass::Spawn,
                format!("could not start declared evaluator: {error}"),
                0,
                0,
            )
        })?;
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(ProcessFailure::new(
            FailureClass::Spawn,
            "child stdout unavailable",
            0,
            0,
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate(&mut child);
        return Err(ProcessFailure::new(
            FailureClass::Spawn,
            "child stderr unavailable",
            0,
            0,
        ));
    };

    let (sender, receiver) = mpsc::channel();
    if let Some(request_bytes) = request_bytes {
        let Some(stdin) = child.stdin.take() else {
            terminate(&mut child);
            return Err(ProcessFailure::new(
                FailureClass::Spawn,
                "child stdin unavailable",
                0,
                0,
            ));
        };
        let stdin_sender = sender.clone();
        std::thread::spawn(move || {
            let mut stdin = stdin;
            let result = stdin.write_all(&request_bytes).map_err(|_| ());
            let _ = stdin_sender.send(StreamEvent::Stdin(result));
        });
    } else {
        let _ = sender.send(StreamEvent::Stdin(Ok(())));
    }
    capture(stdout, limits.max_stdout_bytes, sender.clone(), true);
    capture(stderr, limits.max_stderr_bytes, sender, false);

    Ok((child, receiver))
}

fn collect_process(
    evaluator: &Evaluator,
    child: &mut Child,
    receiver: &mpsc::Receiver<StreamEvent>,
    cancellation: &CancellationToken,
) -> Result<ProcessCapture, ProcessFailure> {
    let started = Instant::now();
    let timeout = Duration::from_secs(evaluator.command().timeout_seconds());
    let mut stdin_done = None;
    let mut stdout_data = None;
    let mut stderr_data = None;
    let mut exit_status = None;
    loop {
        let stdout_count = stdout_data.as_ref().map_or(0, Vec::len);
        let stderr_count = stderr_data.as_ref().map_or(0, Vec::len);
        if cancellation.is_cancelled() {
            terminate(child);
            return Err(ProcessFailure::new(
                FailureClass::Cancelled,
                "evaluator cancelled and owned child reaped",
                stdout_count,
                stderr_count,
            ));
        }
        if started.elapsed() >= timeout {
            terminate(child);
            return Err(ProcessFailure::new(
                FailureClass::Timeout,
                "evaluator deadline exceeded and owned child reaped",
                stdout_count,
                stderr_count,
            ));
        }

        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(StreamEvent::Stdin(result)) => stdin_done = Some(result),
            Ok(StreamEvent::Stdout(Ok(bytes))) => stdout_data = Some(bytes),
            Ok(StreamEvent::Stderr(Ok(bytes))) => stderr_data = Some(bytes),
            Ok(StreamEvent::Stdout(Err(count))) => {
                terminate(child);
                return Err(ProcessFailure::new(
                    FailureClass::OutputLimit,
                    "evaluator stdout exceeded byte limit",
                    count,
                    stderr_count,
                ));
            }
            Ok(StreamEvent::Stderr(Err(count))) => {
                terminate(child);
                return Err(ProcessFailure::new(
                    FailureClass::OutputLimit,
                    "evaluator stderr exceeded byte limit",
                    stdout_count,
                    count,
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected)
                if stdin_done.is_some() && stdout_data.is_some() && stderr_data.is_some() =>
            {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                terminate(child);
                return Err(ProcessFailure::new(
                    FailureClass::Protocol,
                    "evaluator stream capture disconnected",
                    stdout_count,
                    stderr_count,
                ));
            }
        }
        if exit_status.is_none() {
            exit_status = if let Ok(status) = child.try_wait() {
                status
            } else {
                terminate(child);
                return Err(ProcessFailure::new(
                    FailureClass::Spawn,
                    "could not observe evaluator exit",
                    stdout_count,
                    stderr_count,
                ));
            };
        }
        match (
            stdin_done.take(),
            stdout_data.take(),
            stderr_data.take(),
            exit_status,
        ) {
            (Some(stdin_result), Some(stdout), Some(stderr), Some(status)) => {
                return Ok(ProcessCapture {
                    status,
                    stdin_result,
                    stdout,
                    stderr,
                });
            }
            (stdin, stdout, stderr, _) => {
                stdin_done = stdin;
                stdout_data = stdout;
                stderr_data = stderr;
            }
        }
    }
}

fn finish_process(
    context: &EvaluationContext,
    evaluator: &Evaluator,
    captured: ProcessCapture,
) -> Result<ProcessEvaluation, ProcessFailure> {
    let ProcessCapture {
        status,
        stdin_result,
        stdout,
        stderr,
    } = captured;
    let diagnostics = ProcessDiagnostics::new(stdout.len(), stderr.len());
    if !status.success() {
        return Err(ProcessFailure::new(
            FailureClass::NonZeroExit,
            format!("evaluator exited with code {:?}", status.code()),
            stdout.len(),
            stderr.len(),
        ));
    }
    if stdin_result.is_err() {
        return Err(ProcessFailure::new(
            FailureClass::Protocol,
            "evaluator did not accept request",
            stdout.len(),
            stderr.len(),
        ));
    }
    let response = decode_response(&stdout).map_err(|error| {
        ProcessFailure::new(
            FailureClass::Protocol,
            format!("invalid evaluator response: {error}"),
            stdout.len(),
            stderr.len(),
        )
    })?;
    match response.result {
        ProtocolResult::Success { output } => {
            let output = validate_output(context, evaluator.id(), output).map_err(|_| {
                ProcessFailure::new(
                    FailureClass::Validation,
                    "invalid evaluator output; detail redacted",
                    stdout.len(),
                    stderr.len(),
                )
            })?;
            Ok(ProcessEvaluation {
                output,
                diagnostics,
            })
        }
        ProtocolResult::Failure { failure } if failure.class == FailureClass::Reported => {
            Err(ProcessFailure::new(
                FailureClass::Reported,
                "evaluator reported failure; detail redacted",
                stdout.len(),
                stderr.len(),
            ))
        }
        ProtocolResult::Failure { .. } => Err(ProcessFailure::new(
            FailureClass::Protocol,
            "evaluator reported reserved process failure class",
            stdout.len(),
            stderr.len(),
        )),
    }
}

fn capture(
    reader: impl Read + Send + 'static,
    limit: usize,
    sender: mpsc::Sender<StreamEvent>,
    stdout: bool,
) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    let event = if stdout {
                        StreamEvent::Stdout(Ok(bytes))
                    } else {
                        StreamEvent::Stderr(Ok(bytes))
                    };
                    let _ = sender.send(event);
                    return;
                }
                Ok(count) if count > limit.saturating_sub(bytes.len()) => {
                    let observed = bytes.len().saturating_add(count);
                    let event = if stdout {
                        StreamEvent::Stdout(Err(observed))
                    } else {
                        StreamEvent::Stderr(Err(observed))
                    };
                    let _ = sender.send(event);
                    return;
                }
                Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                Err(_) => {
                    let event = if stdout {
                        StreamEvent::Stdout(Err(limit.saturating_add(1)))
                    } else {
                        StreamEvent::Stderr(Err(limit.saturating_add(1)))
                    };
                    let _ = sender.send(event);
                    return;
                }
            }
        }
    });
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
