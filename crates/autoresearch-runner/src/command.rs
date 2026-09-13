//! Local command mutation adapter. No shell, ambient environment, or raw logs.

use crate::{MutationRequest, submit_manual_candidate};
use autoresearch_config::{CommandSpec, ValidatedManifest};
use autoresearch_core::{CandidateCommit, CandidateWorkspace, FailureClass, RunWorkspace};
use autoresearch_evaluator::CancellationToken;
use autoresearch_git::{GitError, LockedGitRepository};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Redacted process outcome plus contained candidate commit.
#[derive(Debug)]
pub struct MutationCommandReport {
    /// Exact Git commit from Phase 2 containment adapter.
    pub candidate_commit: CandidateCommit,
    /// Captured stdout byte count; raw output is never returned.
    pub stdout_bytes: usize,
    /// Captured stderr byte count; raw output is never returned.
    pub stderr_bytes: usize,
}

/// Typed command or containment refusal; no raw process output.
#[derive(Debug, Error)]
pub enum MutationCommandError {
    /// No executable is explicitly allowed.
    #[error("declared mutation executable is not allowlisted")]
    ExecutableNotAllowed,
    /// This adapter has no per-run external-action authority path.
    #[error("command mutation requires default-denied external authority")]
    ExternalAuthorityUnsupported,
    /// Declared command/rubric does not match frozen request manifest.
    #[error("mutation manifest differs from frozen request")]
    RubricMismatch,
    /// Request is not tied to prepared worktree.
    #[error("mutation request worktree does not match prepared candidate")]
    WorktreeMismatch,
    /// JSON request exceeds configured cap.
    #[error("mutation request exceeds byte limit")]
    RequestTooLarge,
    /// Process failed without retaining raw stdout/stderr.
    #[error("mutation process failed: {class:?}: {detail}")]
    Process {
        /// Stable failure class.
        class: FailureClass,
        /// Redacted stable detail.
        detail: &'static str,
    },
    /// Prepared candidate or changed files failed Phase 2 Git checks.
    #[error(transparent)]
    Git(#[from] GitError),
    /// Manual containment check failed after process exit.
    #[error(transparent)]
    Mutation(#[from] crate::MutationError),
    /// Request could not be serialized.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Operator-provided executable allowlist and independent output caps.
#[derive(Debug)]
pub struct CommandMutationAdapter {
    executables: BTreeSet<PathBuf>,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    cancellation: CancellationToken,
}

impl CommandMutationAdapter {
    /// Creates adapter. Empty allowlist means every command is denied.
    ///
    /// # Errors
    ///
    /// Rejects non-absolute, missing, or non-file allowlist entries and zero
    /// output caps. Callers must authorize each executable outside manifest.
    pub fn new(
        allowed_executables: &[PathBuf],
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
        cancellation: CancellationToken,
    ) -> Result<Self, MutationCommandError> {
        if max_stdout_bytes == 0 || max_stderr_bytes == 0 {
            return Err(MutationCommandError::Process {
                class: FailureClass::Validation,
                detail: "output caps must be nonzero",
            });
        }
        let mut executables = BTreeSet::new();
        for path in allowed_executables {
            if !path.is_absolute() {
                return Err(MutationCommandError::ExecutableNotAllowed);
            }
            let canonical =
                fs::canonicalize(path).map_err(|_| MutationCommandError::ExecutableNotAllowed)?;
            if !canonical.is_file() {
                return Err(MutationCommandError::ExecutableNotAllowed);
            }
            executables.insert(canonical);
        }
        Ok(Self {
            executables,
            max_stdout_bytes,
            max_stderr_bytes,
            cancellation,
        })
    }

    /// Runs frozen literal argv in clean candidate worktree, then submits
    /// changes through Phase 2 containment before reporting success.
    ///
    /// # Errors
    ///
    /// Refuses unallowlisted binaries, changed candidate, timeout,
    /// cancellation, output overflow, nonzero exit, or any scope breach.
    pub fn execute_and_commit(
        &self,
        repository: &Path,
        git: &LockedGitRepository<'_>,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        request: &MutationRequest,
        manifest: &ValidatedManifest,
    ) -> Result<MutationCommandReport, MutationCommandError> {
        if !manifest.authority().allowed().is_empty() {
            return Err(MutationCommandError::ExternalAuthorityUnsupported);
        }
        if !request.matches_manifest(manifest) {
            return Err(MutationCommandError::RubricMismatch);
        }
        if request.candidate_worktree() != candidate.path() {
            return Err(MutationCommandError::WorktreeMismatch);
        }
        let command = manifest.agent();
        git.validate_prepared_candidate(run, candidate)?;
        let executable = self.resolve_executable(command.program())?;
        let mut input = serde_json::to_vec(request)?;
        input.push(b'\n');
        if input.len() > MAX_REQUEST_BYTES {
            return Err(MutationCommandError::RequestTooLarge);
        }
        if self.cancellation.is_cancelled() {
            return Err(process_error(
                FailureClass::Cancelled,
                "cancelled before mutation",
            ));
        }
        let captured = self.run_process(&executable, command, candidate.path(), input)?;
        let commit = submit_manual_candidate(repository, git, run, candidate, request)?;
        Ok(MutationCommandReport {
            candidate_commit: commit,
            stdout_bytes: captured.0,
            stderr_bytes: captured.1,
        })
    }

    fn resolve_executable(&self, program: &str) -> Result<PathBuf, MutationCommandError> {
        let path = Path::new(program);
        if !path.is_absolute() {
            return Err(MutationCommandError::ExecutableNotAllowed);
        }
        let canonical =
            fs::canonicalize(path).map_err(|_| MutationCommandError::ExecutableNotAllowed)?;
        if self.executables.contains(&canonical) {
            Ok(canonical)
        } else {
            Err(MutationCommandError::ExecutableNotAllowed)
        }
    }

    fn run_process(
        &self,
        executable: &Path,
        command: &CommandSpec,
        worktree: &Path,
        input: Vec<u8>,
    ) -> Result<(usize, usize), MutationCommandError> {
        let mut child = Command::new(executable)
            .args(command.args())
            .current_dir(worktree)
            .env_clear()
            .env("LANG", "C")
            .env("TZ", "UTC")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| process_error(FailureClass::Spawn, "could not start mutation process"))?;
        let (sender, receiver) = mpsc::channel();
        if let Some(stdin) = child.stdin.take() {
            let writer = sender.clone();
            std::thread::spawn(move || {
                let mut stdin = stdin;
                let ok = stdin.write_all(&input).is_ok();
                let _ = writer.send(StreamEvent::Input(ok));
            });
        } else {
            terminate(&mut child);
            return Err(process_error(
                FailureClass::Spawn,
                "mutation stdin unavailable",
            ));
        }
        if let Some(stdout) = child.stdout.take() {
            capture_stream(stdout, self.max_stdout_bytes, sender.clone(), true);
        } else {
            terminate(&mut child);
            return Err(process_error(
                FailureClass::Spawn,
                "mutation stdout unavailable",
            ));
        }
        if let Some(stderr) = child.stderr.take() {
            capture_stream(stderr, self.max_stderr_bytes, sender, false);
        } else {
            terminate(&mut child);
            return Err(process_error(
                FailureClass::Spawn,
                "mutation stderr unavailable",
            ));
        }
        self.collect(
            &mut child,
            &receiver,
            Duration::from_secs(command.timeout_seconds()),
        )
    }

    fn collect(
        &self,
        child: &mut Child,
        receiver: &Receiver<StreamEvent>,
        timeout: Duration,
    ) -> Result<(usize, usize), MutationCommandError> {
        let started = Instant::now();
        let mut input_ok = None;
        let mut stdout = None;
        let mut stderr = None;
        loop {
            if self.cancellation.is_cancelled() {
                terminate(child);
                return Err(process_error(FailureClass::Cancelled, "mutation cancelled"));
            }
            if started.elapsed() >= timeout {
                terminate(child);
                return Err(process_error(FailureClass::Timeout, "mutation timed out"));
            }
            if input_ok.is_some() && stdout.is_some() && stderr.is_some() {
                let status = child.try_wait().map_err(|_| {
                    process_error(FailureClass::Spawn, "could not wait for mutation")
                })?;
                if let Some(status) = status {
                    if !status.success() {
                        return Err(process_error(
                            FailureClass::NonZeroExit,
                            "mutation exited unsuccessfully",
                        ));
                    }
                    if input_ok != Some(true) {
                        return Err(process_error(
                            FailureClass::Protocol,
                            "mutation did not read request",
                        ));
                    }
                    return Ok((stdout.flatten().unwrap_or(0), stderr.flatten().unwrap_or(0)));
                }
            }
            match receiver.recv_timeout(POLL_INTERVAL) {
                Ok(StreamEvent::Input(ok)) => input_ok = Some(ok),
                Ok(StreamEvent::Stdout(result)) => stdout = Some(result),
                Ok(StreamEvent::Stderr(result)) => stderr = Some(result),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if input_ok.is_none() || stdout.is_none() || stderr.is_none() {
                        terminate(child);
                        return Err(process_error(
                            FailureClass::Protocol,
                            "mutation streams disconnected",
                        ));
                    }
                }
            }
            if stdout == Some(None) || stderr == Some(None) {
                terminate(child);
                return Err(process_error(
                    FailureClass::OutputLimit,
                    "mutation output exceeded cap",
                ));
            }
        }
    }
}

#[derive(Debug)]
enum StreamEvent {
    Input(bool),
    Stdout(Option<usize>),
    Stderr(Option<usize>),
}

fn capture_stream(
    mut stream: impl Read + Send + 'static,
    limit: usize,
    sender: Sender<StreamEvent>,
    stdout: bool,
) {
    std::thread::spawn(move || {
        let mut total = 0_usize;
        let mut buffer = [0_u8; 8192];
        let result = loop {
            match stream.read(&mut buffer) {
                Ok(0) => break Some(total),
                Ok(bytes) => {
                    total = total.saturating_add(bytes);
                    if total > limit {
                        break None;
                    }
                }
                Err(_) => break None,
            }
        };
        let _ = sender.send(if stdout {
            StreamEvent::Stdout(result)
        } else {
            StreamEvent::Stderr(result)
        });
    });
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn process_error(class: FailureClass, detail: &'static str) -> MutationCommandError {
    MutationCommandError::Process { class, detail }
}
