//! Exact-commit baseline evaluation for a previously frozen run.

use autoresearch_config::{
    Evaluator, FrozenIdentity, IdentityError, ManifestError, ValidatedManifest,
};
use autoresearch_core::{
    Complexity, DecisionError, EvaluationSnapshot, EvaluatorFailure, FailureClass, JournalEntry,
    JournalError, JournalEvent, RecoveryAction, ReplayState, RepoPath, RepositoryInspector, RunId,
    RunView, replay_journal,
};
use autoresearch_evaluator::{
    CancellationToken, ContextError, EvaluationContext, EvaluationContextSpec, OutputError,
    ProcessLimitError, ProcessLimits, ValidatedOutput, ValidationError, build_snapshot,
    evaluate_subprocess,
};
use autoresearch_git::{GitError, GitRepository, LockedGitRepository, RunLockGuard};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

const MAX_FROZEN_BYTES: u64 = 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;

/// Runner infrastructure or frozen-state failure. Evaluator failure is a
/// separate journalled outcome, never a fabricated score.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// Repository, lock, or worktree safety failure.
    #[error(transparent)]
    Git(#[from] GitError),
    /// Frozen manifest cannot be parsed.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Frozen identity cannot be computed.
    #[error(transparent)]
    Identity(#[from] IdentityError),
    /// Evaluator context cannot be constructed.
    #[error(transparent)]
    Context(#[from] ContextError),
    /// Journal history is invalid.
    #[error(transparent)]
    Journal(#[from] JournalError),
    /// Frozen lexicographic policy cannot compare snapshots.
    #[error(transparent)]
    Decision(#[from] DecisionError),
    /// Evaluator outputs cannot form complete frozen snapshot.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// Replayed evaluator envelope failed shared structural validation.
    #[error(transparent)]
    Output(#[from] OutputError),
    /// Mutation request or manual containment failed.
    #[error(transparent)]
    Mutation(#[from] crate::MutationError),
    /// Allowlisted command mutation failed.
    #[error(transparent)]
    MutationCommand(#[from] crate::MutationCommandError),
    /// Candidate evaluator failed without a comparable score.
    #[error("candidate evaluator {evaluator_id} failed: {failure:?}")]
    CandidateEvaluator {
        /// Frozen evaluator ID.
        evaluator_id: String,
        /// Redacted typed failure.
        failure: EvaluatorFailure,
    },
    /// Stored JSON cannot be parsed or written.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Read or append failed for one run-owned path.
    #[error("run state I/O failed for {}: {source}", path.display())]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Frozen state differs from journal or is unsafe to continue.
    #[error("frozen run state invalid: {0}")]
    InvalidState(&'static str),
    /// Run ID is not safe as a path/ref component.
    #[error("invalid run identifier")]
    InvalidRunId,
}

/// Baseline outcome: genuine typed snapshot or explicit evaluator failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineCapture {
    /// Every declared evaluator passed shared validation and snapshot matching.
    Captured {
        /// Frozen run ID.
        run_id: String,
        /// Exact baseline commit snapshot.
        snapshot: EvaluationSnapshot,
    },
    /// Run stopped before mutation; no baseline score exists.
    Failed {
        /// Frozen run ID.
        run_id: String,
        /// Declared evaluator, or snapshot validator when outputs cannot match.
        evaluator_id: String,
        /// Typed redacted failure recorded in journal.
        failure: EvaluatorFailure,
    },
}

/// Evaluates one frozen evaluator against exact context and returns Phase 3
/// validated output. Runner chooses evaluators only from frozen manifest.
pub trait BaselineExecutor {
    /// Runs one declared evaluator.
    ///
    /// # Errors
    ///
    /// Returns typed failure without a fabricated output.
    fn evaluate(
        &self,
        context: &EvaluationContext,
        evaluator: &Evaluator,
    ) -> Result<ValidatedOutput, EvaluatorFailure>;
}

/// Process-based baseline executor using frozen JSONL v1 command contract.
#[derive(Debug)]
pub struct SubprocessBaselineExecutor {
    limits: ProcessLimits,
    cancellation: CancellationToken,
}

impl SubprocessBaselineExecutor {
    /// Constructs non-zero independent stdout and stderr byte caps.
    ///
    /// # Errors
    ///
    /// Rejects zero byte caps.
    pub fn new(
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
        cancellation: CancellationToken,
    ) -> Result<Self, ProcessLimitError> {
        Ok(Self {
            limits: ProcessLimits::new(max_stdout_bytes, max_stderr_bytes)?,
            cancellation,
        })
    }
}

impl BaselineExecutor for SubprocessBaselineExecutor {
    fn evaluate(
        &self,
        context: &EvaluationContext,
        evaluator: &Evaluator,
    ) -> Result<ValidatedOutput, EvaluatorFailure> {
        evaluate_subprocess(context, evaluator, self.limits, &self.cancellation)
            .map(|result| result.output)
            .map_err(|failure| failure.failure)
    }
}

/// Evaluates only frozen declared evaluators in an isolated exact-base worktree.
///
/// The run must have been opened by CLI `baseline`, leaving journal at
/// `run_started`. Success appends `baseline_captured`; evaluator or manifest
/// output failure appends typed `baseline_failed` and stops run. Caller checkout
/// and product gate are never mutated. No agent command runs.
///
/// # Errors
///
/// Returns infrastructure, identity, lock, or unsafe-state failure. Such
/// errors leave baseline pending instead of reporting a pass.
pub fn capture_existing_baseline(
    repository: &Path,
    run_id: &str,
    executor: &impl BaselineExecutor,
    declared_environment: BTreeMap<String, String>,
) -> Result<BaselineCapture, RunnerError> {
    let mut stored = StoredRun::load(repository, run_id)?;
    if stored.view.recovery_action() != &RecoveryAction::CaptureBaseline {
        return Err(RunnerError::InvalidState(
            "run is not awaiting baseline evaluation",
        ));
    }
    let snapshot = GitRepository.inspect(&stored.root, &stored.base_commit)?;
    let lock = RunLockGuard::acquire(&snapshot, run_id)?;
    let git = LockedGitRepository::new(&snapshot, &lock)?;
    let run = git.open_run()?;
    if run.base_commit().as_str() != stored.base_commit || run.head_commit() != run.base_commit() {
        return Err(RunnerError::InvalidState(
            "run branch differs from frozen baseline",
        ));
    }
    let baseline_worktree = git.prepare_baseline(&run)?;
    let artifacts = stored.run_directory.join("artifacts");
    ensure_real_directory(&artifacts)?;
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: run_id.into(),
        baseline_commit: stored.base_commit.clone(),
        evaluated_commit: stored.base_commit.clone(),
        candidate_worktree: baseline_worktree,
        changed_paths: Vec::new(),
        declared_environment,
        artifact_directory: artifacts,
        cancellation_id: format!("{run_id}-baseline"),
    })?;
    let mut outputs = Vec::with_capacity(stored.manifest.evaluators().len());
    let evaluators = stored.manifest.evaluators().to_vec();
    for evaluator in &evaluators {
        let result = executor.evaluate(&context, evaluator);
        if git.prepare_baseline(&run).is_err() {
            return stored.fail(
                evaluator.id(),
                EvaluatorFailure {
                    class: FailureClass::Validation,
                    detail: "baseline worktree changed during evaluator".into(),
                },
            );
        }
        match result {
            Ok(output) => outputs.push(output),
            Err(failure) => return stored.fail(evaluator.id(), redact_failure(&failure)),
        }
    }
    let Ok(snapshot) = build_snapshot(&context, &stored.manifest, outputs, Complexity::default())
    else {
        return stored.fail(
            "snapshot_validation",
            EvaluatorFailure {
                class: FailureClass::Validation,
                detail: "declared evaluator outputs did not form baseline snapshot".into(),
            },
        );
    };
    stored.append(JournalEvent::BaselineCaptured {
        snapshot: snapshot.clone(),
    })?;
    Ok(BaselineCapture::Captured {
        run_id: run_id.into(),
        snapshot,
    })
}

pub(crate) fn redact_failure(failure: &EvaluatorFailure) -> EvaluatorFailure {
    EvaluatorFailure {
        class: failure.class,
        detail: "declared evaluator failed; raw process output withheld".into(),
    }
}

pub(crate) struct StoredRun {
    pub(crate) root: PathBuf,
    pub(crate) run_directory: PathBuf,
    pub(crate) manifest: ValidatedManifest,
    pub(crate) program: String,
    pub(crate) identity: FrozenIdentity,
    pub(crate) base_commit: String,
    pub(crate) entries: Vec<JournalEntry>,
    pub(crate) view: RunView,
}

impl StoredRun {
    pub(crate) fn load(repository: &Path, run_id: &str) -> Result<Self, RunnerError> {
        RunId::new(run_id.to_owned()).map_err(|_| RunnerError::InvalidRunId)?;
        let initial = GitRepository.inspect(repository, "HEAD")?;
        let root = initial.root().to_path_buf();
        let run_directory = root.join(".autoresearch/runs").join(run_id);
        let canonical = fs::canonicalize(&run_directory).map_err(|source| RunnerError::Io {
            path: run_directory.clone(),
            source,
        })?;
        if canonical != run_directory || !canonical.is_dir() {
            return Err(RunnerError::InvalidState(
                "run directory escaped repository",
            ));
        }
        let journal_path = run_directory.join("journal.jsonl");
        let journal = read_regular(&journal_path, MAX_JOURNAL_BYTES)?;
        if !journal.ends_with(b"\n") {
            return Err(RunnerError::InvalidState(
                "journal has incomplete last record",
            ));
        }
        let entries = journal
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice::<JournalEntry>)
            .collect::<Result<Vec<_>, _>>()?;
        let ReplayState::Run(view) = replay_journal(&entries)? else {
            return Err(RunnerError::InvalidState("run journal is empty"));
        };
        if view.run_id() != run_id {
            return Err(RunnerError::InvalidState("run ID differs from journal"));
        }
        let frozen = run_directory.join("frozen");
        let manifest_source = read_regular(&frozen.join("autoresearch.toml"), MAX_FROZEN_BYTES)?;
        let manifest = ValidatedManifest::parse(
            std::str::from_utf8(&manifest_source)
                .map_err(|_| RunnerError::InvalidState("frozen manifest is not UTF-8"))?,
        )?;
        let program = read_regular(&frozen.join("program.md"), MAX_FROZEN_BYTES)?;
        let gate_path = frozen.join("product-gate.md");
        let gate = match fs::symlink_metadata(&gate_path) {
            Ok(_) => Some(read_regular(&gate_path, MAX_FROZEN_BYTES)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(RunnerError::Io {
                    path: gate_path,
                    source,
                });
            }
        };
        let computed =
            FrozenIdentity::capture(&manifest, &program, &BTreeMap::new(), gate.as_deref())?;
        let stored: FrozenIdentity = serde_json::from_slice(&read_regular(
            &run_directory.join("identity.json"),
            MAX_FROZEN_BYTES,
        )?)?;
        if computed != stored || computed.aggregate_sha256 != view.frozen_identity() {
            return Err(RunnerError::InvalidState(
                "frozen identity differs from journal",
            ));
        }
        let base = GitRepository.inspect(&root, view.base_commit())?;
        if base.head_commit().as_str() != view.base_commit() {
            return Err(RunnerError::InvalidState(
                "caller HEAD differs from frozen base commit",
            ));
        }
        for (source, frozen_bytes) in [
            ("autoresearch.toml", Some(manifest_source.as_slice())),
            ("program.md", Some(program.as_slice())),
            ("docs/BET.md", gate.as_deref()),
        ] {
            let path = RepoPath::new(source)
                .map_err(|_| RunnerError::InvalidState("invalid frozen source path"))?;
            let committed = GitRepository.read_blob_at_commit(&base, &path, MAX_FROZEN_BYTES)?;
            if committed.as_deref() != frozen_bytes {
                return Err(RunnerError::InvalidState(
                    "frozen source differs from base commit",
                ));
            }
        }
        Ok(Self {
            root,
            run_directory,
            manifest,
            program: String::from_utf8(program)
                .map_err(|_| RunnerError::InvalidState("frozen program is not UTF-8"))?,
            identity: stored,
            base_commit: view.base_commit().into(),
            entries,
            view: *view,
        })
    }

    fn fail(
        &mut self,
        evaluator_id: &str,
        failure: EvaluatorFailure,
    ) -> Result<BaselineCapture, RunnerError> {
        self.append(JournalEvent::BaselineFailed {
            evaluator_id: evaluator_id.into(),
            failure: failure.clone(),
        })?;
        Ok(BaselineCapture::Failed {
            run_id: self.entries[0].run_id.clone(),
            evaluator_id: evaluator_id.into(),
            failure,
        })
    }

    pub(crate) fn append(&mut self, event: JournalEvent) -> Result<(), RunnerError> {
        let sequence = u64::try_from(self.entries.len())
            .map_err(|_| RunnerError::InvalidState("journal sequence overflow"))?;
        let entry = JournalEntry {
            sequence,
            run_id: self.entries[0].run_id.clone(),
            event,
        };
        let mut projected = self.entries.clone();
        projected.push(entry.clone());
        let ReplayState::Run(projected_view) = replay_journal(&projected)? else {
            return Err(RunnerError::InvalidState("appended journal became empty"));
        };
        let path = self.run_directory.join("journal.jsonl");
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|source| RunnerError::Io {
                path: path.clone(),
                source,
            })?;
        let mut encoded = serde_json::to_vec(&entry)?;
        encoded.push(b'\n');
        file.write_all(&encoded).map_err(|source| RunnerError::Io {
            path: path.clone(),
            source,
        })?;
        file.sync_data()
            .map_err(|source| RunnerError::Io { path, source })?;
        self.entries = projected;
        self.view = *projected_view;
        Ok(())
    }
}

fn read_regular(path: &Path, max_bytes: u64) -> Result<Vec<u8>, RunnerError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RunnerError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes {
        return Err(RunnerError::InvalidState(
            "run input is unsafe or exceeds byte limit",
        ));
    }
    fs::read(path).map_err(|source| RunnerError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn ensure_real_directory(path: &Path) -> Result<(), RunnerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| RunnerError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        Ok(_) => Err(RunnerError::InvalidState(
            "artifact directory is not a real directory",
        )),
        Err(source) => Err(RunnerError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}
