//! Safe foundation commands for autoresearch repositories.

mod baseline;
mod template;

use autoresearch_config::{IdentityError, ManifestError, ValidatedManifest};
use autoresearch_core::{
    CandidateDecision, CandidateFinalization, EvaluationSnapshot, EvaluatorFailure,
};
use autoresearch_evaluator::CancellationToken;
use autoresearch_runner::{
    BaselineCapture, ResumeOutcome, RunMutationMode, RunStatus, RunStep, RunnerError,
    SubprocessBaselineExecutor, VerificationEvidence, advance_run_once, capture_existing_baseline,
    inspect_run, resume_run, stop_run, verify_kept_commit,
};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode, Output};
use thiserror::Error;

const EXIT_CONFIG: u8 = 3;
const EXIT_ENVIRONMENT: u8 = 4;
const EXIT_FAILURE: u8 = 5;

/// Parsed command-line interface.
#[derive(Debug, Parser)]
#[command(name = "autoresearch", version, about)]
pub struct Cli {
    /// Target repository; defaults to current directory.
    #[arg(long, global = true, default_value = ".")]
    repository: PathBuf,
    /// Emit one JSON document instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Foundation command.
    #[command(subcommand)]
    command: Action,
}

/// Implemented foundation commands.
#[derive(Debug, Subcommand)]
enum Action {
    /// Create missing repository contract files without overwriting.
    Init,
    /// Validate repository, contract, and local executables without running them.
    Doctor,
    /// Freeze clean inputs, then evaluate declared baseline against exact commit.
    Baseline,
    /// Advance one bounded candidate in isolated run worktree.
    Run {
        /// Existing frozen run identifier from baseline report.
        #[arg(long)]
        run_id: String,
        /// Manual edits or explicitly allowlisted local command.
        #[arg(long, value_enum, default_value_t = CliMutationMode::Manual)]
        mode: CliMutationMode,
        /// Testable candidate hypothesis; required when submitting mutation.
        #[arg(long)]
        hypothesis: Option<String>,
        /// Exact absolute executable authorized for command mode only.
        #[arg(long)]
        allow_executable: Option<PathBuf>,
    },
    /// Recover exactly next journal action without changing frozen policy.
    Resume {
        /// Existing frozen run identifier.
        #[arg(long)]
        run_id: String,
    },
    /// Inspect frozen run and journal without writing any state.
    Status {
        /// Existing frozen run identifier.
        #[arg(long)]
        run_id: String,
    },
    /// Rerun frozen evaluators at finalized kept commit, without reselection.
    Verify {
        /// Existing frozen run identifier.
        #[arg(long)]
        run_id: String,
    },
    /// Record operator cancellation between candidates.
    Stop {
        /// Existing frozen run identifier.
        #[arg(long)]
        run_id: String,
    },
}

/// Operator-selected mutation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliMutationMode {
    Manual,
    Command,
}

/// Successful command output.
#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CommandReport {
    /// Init result.
    Init(InitReport),
    /// Doctor result.
    Doctor(DoctorReport),
    /// Baseline freeze result.
    Baseline(BaselineReport),
    /// One bounded runner transition.
    Run(RunReport),
    /// Journal-directed recovery transition.
    Resume(ResumeReport),
    /// Read-only journal projection.
    Status(RunStatus),
    /// Independent kept-commit verification evidence.
    Verify(VerificationEvidence),
    /// Operator cancellation at stable boundary.
    Stop(RunStatus),
}

/// Exit status plus report, including diagnostic failure reports.
#[derive(Debug)]
pub struct Execution {
    /// Structured command report.
    pub report: CommandReport,
    /// Process exit code.
    pub exit_code: u8,
}

/// Whether init created or preserved one contract file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InitStatus {
    /// File was absent and created.
    Created,
    /// Existing bytes were left unchanged.
    Preserved,
}

/// Non-destructive init result.
#[derive(Debug, Serialize)]
pub struct InitReport {
    /// Target repository.
    pub repository: PathBuf,
    /// Manifest outcome.
    pub manifest: InitStatus,
    /// Program outcome.
    pub program: InitStatus,
}

/// Named doctor check.
#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: String,
    /// Whether required condition passed.
    pub passed: bool,
    /// Evidence or failure detail.
    pub detail: String,
}

/// Read-only doctor result.
#[derive(Debug, Serialize)]
pub struct DoctorReport {
    /// Target repository.
    pub repository: PathBuf,
    /// True only when every required check passed.
    pub ready: bool,
    /// Ordered checks.
    pub checks: Vec<DoctorCheck>,
}

/// Frozen run and genuine declared-evaluator result.
#[derive(Debug, Serialize)]
pub struct BaselineReport {
    /// Unique run identifier.
    pub run_id: String,
    /// Durable run evidence directory.
    pub run_directory: PathBuf,
    /// Exact baseline Git commit.
    pub base_commit: String,
    /// Aggregate SHA-256 over frozen inputs.
    pub frozen_identity: String,
    /// Replay-required next action.
    pub next_action: String,
    /// Explicit evidence state.
    pub evidence_status: String,
    /// Exact validated baseline snapshot when all evaluators pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<EvaluationSnapshot>,
    /// Frozen evaluator that failed, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_evaluator: Option<String>,
    /// Typed redacted failure, never fabricated score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<EvaluatorFailure>,
}

/// One serial candidate transition with exact evidence if evaluated.
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Frozen run identifier.
    pub run_id: String,
    /// Stable machine-readable transition.
    pub status: String,
    /// Candidate number when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Isolated candidate worktree for manual editing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    /// Exact evaluated candidate commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Complete frozen evaluator snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<EvaluationSnapshot>,
    /// Frozen lexicographic decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<CandidateDecision>,
    /// Confirmed Git finalization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalization: Option<CandidateFinalization>,
    /// Durable stopping reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// One recovered journal action; no guessed rerun or silent decision change.
#[derive(Debug, Serialize)]
pub struct ResumeReport {
    /// Frozen run identifier.
    pub run_id: String,
    /// Stable recovery outcome.
    pub status: String,
    /// Candidate number when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Isolated candidate needing mutation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    /// Exact evaluated candidate commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Captured baseline or candidate evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<EvaluationSnapshot>,
    /// Fresh failure when recovering incomplete baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<EvaluatorFailure>,
    /// Frozen candidate policy result, if evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<CandidateDecision>,
    /// Confirmed Git side effect, if finalized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalization: Option<CandidateFinalization>,
}

/// Command execution failure.
#[derive(Debug, Error)]
pub enum AppError {
    /// Filesystem operation failed.
    #[error("{operation} failed for `{}`: {source}", path.display())]
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Manifest rejected.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Frozen identity construction failed.
    #[error(transparent)]
    Identity(#[from] IdentityError),
    /// JSON report or journal serialization failed.
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Local Git invocation failed.
    #[error("git command failed: {0}")]
    Git(String),
    /// Program file cannot provide research instructions.
    #[error("program.md cannot be blank")]
    BlankProgram,
    /// Baseline inputs must already be committed.
    #[error("frozen inputs have uncommitted changes:\n{0}")]
    DirtyFrozenInputs(String),
    /// Run evidence must stay outside repository changes.
    #[error(".autoresearch/ is not ignored; add it to .gitignore and commit that change")]
    RunStateNotIgnored,
    /// System clock cannot produce sortable run ID.
    #[error("system clock is before Unix epoch")]
    ClockBeforeEpoch,
    /// Unique run directory could not be allocated safely.
    #[error("could not allocate unique run id")]
    RunIdCollision,
    /// Runner refused unsafe or incomplete frozen state.
    #[error(transparent)]
    Runner(#[from] RunnerError),
    /// Evaluator failed after run directory was frozen.
    #[error("baseline run {run_id} could not evaluate: {source}")]
    BaselineExecution {
        /// Frozen run identifier for explicit recovery.
        run_id: String,
        /// Runner infrastructure refusal.
        #[source]
        source: RunnerError,
    },
    /// Command mode lacks separate explicit executable authority.
    #[error("command mode requires --allow-executable with exact absolute binary path")]
    MissingExecutableAuthority,
    /// Manual mode cannot accept command executable authority.
    #[error("--allow-executable applies only to command mode")]
    UnexpectedExecutableAuthority,
}

impl AppError {
    const fn exit_code(&self) -> u8 {
        match self {
            Self::Manifest(_)
            | Self::Identity(_)
            | Self::MissingExecutableAuthority
            | Self::UnexpectedExecutableAuthority => EXIT_CONFIG,
            Self::Runner(RunnerError::CandidateEvaluator { .. }) => EXIT_FAILURE,
            Self::Git(_)
            | Self::BlankProgram
            | Self::DirtyFrozenInputs(_)
            | Self::RunStateNotIgnored
            | Self::Runner(_)
            | Self::BaselineExecution { .. } => EXIT_ENVIRONMENT,
            Self::Io { .. } | Self::Json(_) | Self::ClockBeforeEpoch | Self::RunIdCollision => {
                EXIT_FAILURE
            }
        }
    }
}

/// Parses process arguments, executes command, renders report, and returns exit status.
#[must_use]
pub fn main_entry() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match execute(&cli) {
        Ok(execution) => {
            if let Err(error) = render(&execution.report, json) {
                eprintln!("error: {error}");
                return ExitCode::from(error.exit_code());
            }
            ExitCode::from(execution.exit_code)
        }
        Err(error) => {
            if json {
                let fallback = format!(
                    "{{\"ok\":false,\"exit_code\":{},\"error\":{}}}",
                    error.exit_code(),
                    serde_json::to_string(&error.to_string())
                        .unwrap_or_else(|_| "\"serialization failure\"".into())
                );
                eprintln!("{fallback}");
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::from(error.exit_code())
        }
    }
}

/// Executes parsed CLI without terminating process.
///
/// # Errors
///
/// Returns [`AppError`] for filesystem, Git, config, identity, or serialization failures.
pub fn execute(cli: &Cli) -> Result<Execution, AppError> {
    let report = match &cli.command {
        Action::Init => CommandReport::Init(initialize(&cli.repository)?),
        Action::Doctor => CommandReport::Doctor(doctor(&cli.repository)),
        Action::Baseline => CommandReport::Baseline(capture_baseline(&cli.repository)?),
        Action::Run {
            run_id,
            mode,
            hypothesis,
            allow_executable,
        } => CommandReport::Run(run_once(
            &cli.repository,
            run_id,
            *mode,
            hypothesis.as_deref(),
            allow_executable.as_deref(),
        )?),
        Action::Resume { run_id } => CommandReport::Resume(resume_once(&cli.repository, run_id)?),
        Action::Status { run_id } => CommandReport::Status(inspect_run(&cli.repository, run_id)?),
        Action::Verify { run_id } => CommandReport::Verify(verify_kept_commit(
            &cli.repository,
            run_id,
            &evaluator_executor()?,
        )?),
        Action::Stop { run_id } => CommandReport::Stop(stop_run(&cli.repository, run_id)?),
    };
    let exit_code = match &report {
        CommandReport::Doctor(report) if !report.ready => EXIT_ENVIRONMENT,
        CommandReport::Baseline(report) if report.failure.is_some() => EXIT_FAILURE,
        CommandReport::Resume(report) if report.failure.is_some() => EXIT_FAILURE,
        CommandReport::Verify(report) if report.status != "matched" => EXIT_FAILURE,
        _ => 0,
    };
    Ok(Execution { report, exit_code })
}

fn resume_once(repository: &Path, run_id: &str) -> Result<ResumeReport, AppError> {
    let result = resume_run(repository, run_id, &evaluator_executor()?, BTreeMap::new())?;
    let mut report = ResumeReport {
        run_id: run_id.into(),
        status: String::new(),
        index: None,
        worktree: None,
        commit: None,
        snapshot: None,
        failure: None,
        decision: None,
        finalization: None,
    };
    match result {
        ResumeOutcome::Baseline(BaselineCapture::Captured { snapshot, .. }) => {
            report.status = "baseline_captured".into();
            report.snapshot = Some(snapshot);
        }
        ResumeOutcome::Baseline(BaselineCapture::Failed { failure, .. }) => {
            report.status = "baseline_failed".into();
            report.failure = Some(failure);
        }
        ResumeOutcome::NeedsMutation { index, worktree } => {
            report.status = "needs_mutation".into();
            report.index = Some(index);
            report.worktree = Some(worktree);
        }
        ResumeOutcome::Candidate(outcome) => {
            report.status = "candidate_evaluated".into();
            report.index = Some(outcome.index);
            report.commit = Some(outcome.commit);
            report.snapshot = Some(outcome.snapshot);
            report.decision = Some(outcome.decision);
            report.finalization = Some(outcome.finalization);
        }
        ResumeOutcome::Finalized { index, outcome } => {
            report.status = "candidate_finalized".into();
            report.index = Some(index);
            report.finalization = Some(outcome);
        }
        ResumeOutcome::ReadyForCandidate { index } => {
            report.status = "ready_for_candidate".into();
            report.index = Some(index);
        }
        ResumeOutcome::Finished => report.status = "finished".into(),
    }
    Ok(report)
}

fn evaluator_executor() -> Result<SubprocessBaselineExecutor, AppError> {
    SubprocessBaselineExecutor::new(1024 * 1024, 1024 * 1024, CancellationToken::default())
        .map_err(|_| AppError::Git("evaluator output caps are invalid".into()))
}

fn capture_baseline(repository: &Path) -> Result<BaselineReport, AppError> {
    let mut report = baseline::capture(repository)?;
    let executor = evaluator_executor()?;
    let outcome = capture_existing_baseline(repository, &report.run_id, &executor, BTreeMap::new())
        .map_err(|source| AppError::BaselineExecution {
            run_id: report.run_id.clone(),
            source,
        })?;
    match outcome {
        BaselineCapture::Captured { snapshot, .. } => {
            report.next_action = "run".into();
            report.evidence_status = "captured".into();
            report.snapshot = Some(snapshot);
        }
        BaselineCapture::Failed {
            evaluator_id,
            failure,
            ..
        } => {
            report.next_action = "inspect_baseline_failure".into();
            report.evidence_status = "failed".into();
            report.failed_evaluator = Some(evaluator_id);
            report.failure = Some(failure);
        }
    }
    Ok(report)
}

fn run_once(
    repository: &Path,
    run_id: &str,
    mode: CliMutationMode,
    hypothesis: Option<&str>,
    allow_executable: Option<&Path>,
) -> Result<RunReport, AppError> {
    let mode = match (mode, allow_executable) {
        (CliMutationMode::Manual, None) => RunMutationMode::Manual,
        (CliMutationMode::Manual, Some(_)) => return Err(AppError::UnexpectedExecutableAuthority),
        (CliMutationMode::Command, None) => return Err(AppError::MissingExecutableAuthority),
        (CliMutationMode::Command, Some(executable)) => RunMutationMode::Command {
            executable: executable.to_path_buf(),
        },
    };
    let step = advance_run_once(
        repository,
        run_id,
        &mode,
        hypothesis,
        &evaluator_executor()?,
    )?;
    let mut report = RunReport {
        run_id: run_id.into(),
        status: String::new(),
        index: None,
        worktree: None,
        commit: None,
        snapshot: None,
        decision: None,
        finalization: None,
        stop_reason: None,
    };
    match step {
        RunStep::AwaitingMutation { index, worktree } => {
            report.status = "awaiting_mutation".into();
            report.index = Some(index);
            report.worktree = Some(worktree);
        }
        RunStep::Evaluated(outcome) => {
            report.status = "evaluated".into();
            report.index = Some(outcome.index);
            report.commit = Some(outcome.commit);
            report.snapshot = Some(outcome.snapshot);
            report.decision = Some(outcome.decision);
            report.finalization = Some(outcome.finalization);
        }
        RunStep::Finalized { index, outcome } => {
            report.status = "finalized".into();
            report.index = Some(index);
            report.finalization = Some(outcome);
        }
        RunStep::Stopped { reason } => {
            report.status = "stopped".into();
            report.stop_reason = Some(reason);
        }
    }
    Ok(report)
}

fn initialize(repository: &Path) -> Result<InitReport, AppError> {
    if !repository.is_dir() {
        return Err(AppError::Io {
            operation: "open repository",
            path: repository.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "directory does not exist"),
        });
    }
    let manifest = create_if_missing(&repository.join("autoresearch.toml"), template::MANIFEST)?;
    let program = create_if_missing(&repository.join("program.md"), template::PROGRAM)?;
    Ok(InitReport {
        repository: repository.to_path_buf(),
        manifest,
        program,
    })
}

fn create_if_missing(path: &Path, contents: &str) -> Result<InitStatus, AppError> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(contents.as_bytes())
                .map_err(|source| AppError::Io {
                    operation: "write contract",
                    path: path.to_path_buf(),
                    source,
                })?;
            file.sync_all().map_err(|source| AppError::Io {
                operation: "sync contract",
                path: path.to_path_buf(),
                source,
            })?;
            Ok(InitStatus::Created)
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(InitStatus::Preserved)
        }
        Err(source) => Err(AppError::Io {
            operation: "create contract",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn doctor(repository: &Path) -> DoctorReport {
    let mut checks = Vec::new();
    let root = match git_output(repository, &["rev-parse", "--show-toplevel"]) {
        Ok(root) => {
            push_check(&mut checks, "git_worktree", true, root.clone());
            Some(PathBuf::from(root))
        }
        Err(error) => {
            push_check(&mut checks, "git_worktree", false, error.to_string());
            None
        }
    };
    let effective_root = root.as_deref().unwrap_or(repository);

    let manifest = match fs::read_to_string(effective_root.join("autoresearch.toml")) {
        Ok(source) => match read_manifest(&source) {
            Ok(manifest) => {
                push_check(
                    &mut checks,
                    "manifest",
                    true,
                    format!("schema {} valid", manifest.schema_version()),
                );
                Some(manifest)
            }
            Err(error) => {
                push_check(&mut checks, "manifest", false, error.to_string());
                None
            }
        },
        Err(error) => {
            push_check(&mut checks, "manifest", false, error.to_string());
            None
        }
    };

    match fs::read(effective_root.join("program.md")) {
        Ok(bytes) if !bytes.iter().all(u8::is_ascii_whitespace) => {
            push_check(&mut checks, "program", true, "program.md is nonblank");
        }
        Ok(_) => push_check(&mut checks, "program", false, "program.md is blank"),
        Err(error) => push_check(&mut checks, "program", false, error.to_string()),
    }

    match root {
        Some(_) => match git_output(
            effective_root,
            &["check-ignore", "-q", ".autoresearch/probe"],
        ) {
            Ok(_) => push_check(
                &mut checks,
                "run_state_ignored",
                true,
                ".autoresearch/ is ignored",
            ),
            Err(_) => push_check(
                &mut checks,
                "run_state_ignored",
                false,
                "add .autoresearch/ to .gitignore",
            ),
        },
        None => push_check(
            &mut checks,
            "run_state_ignored",
            false,
            "requires Git worktree",
        ),
    }

    if let Some(manifest) = manifest {
        check_executable(
            &mut checks,
            effective_root,
            "agent_executable",
            manifest.agent().program(),
        );
        for evaluator in manifest.evaluators() {
            check_executable(
                &mut checks,
                effective_root,
                &format!(
                    "evaluator_executable:{}.{}",
                    evaluator.id(),
                    evaluator.command().program()
                ),
                evaluator.command().program(),
            );
        }
    }

    DoctorReport {
        repository: effective_root.to_path_buf(),
        ready: checks.iter().all(|check| check.passed),
        checks,
    }
}

fn check_executable(checks: &mut Vec<DoctorCheck>, root: &Path, name: &str, program: &str) {
    match find_executable(root, program) {
        Some(path) => push_check(checks, name, true, path.display().to_string()),
        None => push_check(
            checks,
            name,
            false,
            format!("`{program}` not found or not executable"),
        ),
    }
}

fn find_executable(root: &Path, program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        let path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        return is_executable(&path).then_some(path);
    }
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|path| path.join(program))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn push_check(checks: &mut Vec<DoctorCheck>, name: &str, passed: bool, detail: impl Into<String>) {
    checks.push(DoctorCheck {
        name: name.to_owned(),
        passed,
        detail: detail.into(),
    });
}

pub(crate) fn read_manifest(source: &str) -> Result<ValidatedManifest, AppError> {
    Ok(ValidatedManifest::parse(source)?)
}

pub(crate) fn git_output(repository: &Path, args: &[&str]) -> Result<String, AppError> {
    let output = ProcessCommand::new("git")
        .arg("-C")
        .arg(repository)
        .args(args.iter().map(OsStr::new))
        .output()
        .map_err(|source| AppError::Io {
            operation: "execute git",
            path: repository.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(AppError::Git(command_failure(&output)))
    }
}

fn command_failure(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stderr
    }
}

fn render(report: &CommandReport, json: bool) -> Result<(), AppError> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    match report {
        CommandReport::Init(report) => {
            println!("repository: {}", report.repository.display());
            println!("autoresearch.toml: {}", status_text(report.manifest));
            println!("program.md: {}", status_text(report.program));
        }
        CommandReport::Doctor(report) => {
            println!("ready: {}", report.ready);
            for check in &report.checks {
                let status = if check.passed { "pass" } else { "fail" };
                println!("{status} {}: {}", check.name, check.detail);
            }
        }
        CommandReport::Baseline(report) => {
            println!("run: {}", report.run_id);
            println!("base commit: {}", report.base_commit);
            println!("frozen identity: {}", report.frozen_identity);
            println!("evidence: {}", report.evidence_status);
            println!("next: {}", report.next_action);
            println!("directory: {}", report.run_directory.display());
            if let Some(snapshot) = &report.snapshot {
                println!("snapshot: {}", serde_json::to_string(snapshot)?);
            }
            if let Some(failure) = &report.failure {
                println!(
                    "failed evaluator: {}",
                    report.failed_evaluator.as_deref().unwrap_or("unknown")
                );
                println!("failure: {}", serde_json::to_string(failure)?);
            }
        }
        CommandReport::Run(report) => {
            println!("run: {}", report.run_id);
            println!("status: {}", report.status);
            if let Some(worktree) = &report.worktree {
                println!("worktree: {}", worktree.display());
            }
            if let Some(commit) = &report.commit {
                println!("commit: {commit}");
            }
            if let Some(reason) = &report.stop_reason {
                println!("stop: {reason}");
            }
        }
        CommandReport::Resume(report) => {
            println!("run: {}", report.run_id);
            println!("status: {}", report.status);
            if let Some(index) = report.index {
                println!("candidate: {index}");
            }
            if let Some(worktree) = &report.worktree {
                println!("worktree: {}", worktree.display());
            }
        }
        CommandReport::Status(report) | CommandReport::Stop(report) => {
            println!("run: {}", report.run_id);
            println!("current commit: {}", report.current_commit);
            println!("recovery: {}", report.recovery_action);
            if let Some(reason) = &report.stop_reason {
                println!("stop: {reason}");
            }
        }
        CommandReport::Verify(report) => {
            println!("run: {}", report.run_id);
            println!("verified commit: {}", report.verified_commit);
            println!("fresh verification: {}", report.status);
            println!("evidence: {}", report.evidence_path.display());
        }
    }
    Ok(())
}

const fn status_text(status: InitStatus) -> &'static str {
    match status {
        InitStatus::Created => "created",
        InitStatus::Preserved => "preserved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock")
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "autoresearch-rs-test-{}-{nanos}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if self
                .0
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("autoresearch-rs-test-"))
            {
                fs::remove_dir_all(&self.0).expect("remove test directory");
            }
        }
    }

    #[test]
    fn init_is_idempotent_and_preserves_existing_bytes() {
        let directory = TestDirectory::new();
        let first = initialize(&directory.0).expect("first init");
        assert_eq!(first.manifest, InitStatus::Created);
        assert_eq!(first.program, InitStatus::Created);
        let manifest = fs::read(directory.0.join("autoresearch.toml")).expect("read manifest");
        let program_path = directory.0.join("program.md");
        fs::write(&program_path, "custom program\n").expect("customize program");

        let second = initialize(&directory.0).expect("second init");
        assert_eq!(second.manifest, InitStatus::Preserved);
        assert_eq!(second.program, InitStatus::Preserved);
        assert_eq!(
            fs::read(directory.0.join("autoresearch.toml")).expect("read manifest"),
            manifest
        );
        assert_eq!(
            fs::read_to_string(program_path).expect("read program"),
            "custom program\n"
        );
    }

    #[test]
    fn doctor_reports_missing_requirements_without_running_commands() {
        let directory = TestDirectory::new();
        let report = doctor(&directory.0);
        assert!(!report.ready);
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "git_worktree" && !check.passed)
        );
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "manifest" && !check.passed)
        );
    }
}
