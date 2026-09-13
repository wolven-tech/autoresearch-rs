//! Exact revision and path identity carried into one evaluator invocation.

use autoresearch_core::{
    CommitId, RepoPath, RepoPathError, RepositoryValueError, RunId, WorkspaceValueError,
};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Unvalidated caller-supplied values for one evaluator invocation.
#[derive(Debug, Clone)]
pub struct EvaluationContextSpec {
    /// Stable run ID, safe as one filesystem component.
    pub run_id: String,
    /// Exact baseline Git object ID.
    pub baseline_commit: String,
    /// Exact Git object ID being evaluated.
    pub evaluated_commit: String,
    /// Existing isolated candidate worktree directory.
    pub candidate_worktree: PathBuf,
    /// Repository-relative changed paths.
    pub changed_paths: Vec<String>,
    /// Only environment values explicitly declared for this invocation.
    pub declared_environment: BTreeMap<String, String>,
    /// Existing run-owned artifact directory, separate from worktree.
    pub artifact_directory: PathBuf,
    /// Stable cancellation token identifier.
    pub cancellation_id: String,
}

/// Invalid evaluator invocation context.
#[derive(Debug, Error)]
pub enum ContextError {
    /// Run identifier is unsafe.
    #[error("invalid run ID: {0}")]
    RunId(WorkspaceValueError),
    /// Baseline commit is not a full object ID.
    #[error("invalid baseline commit: {0}")]
    BaselineCommit(RepositoryValueError),
    /// Evaluated commit is not a full object ID.
    #[error("invalid evaluated commit: {0}")]
    EvaluatedCommit(RepositoryValueError),
    /// Candidate worktree is not a canonical, existing directory.
    #[error("unsafe candidate worktree: {0}")]
    CandidateWorktree(String),
    /// Artifact root is not a canonical, existing directory.
    #[error("unsafe artifact directory: {0}")]
    ArtifactDirectory(String),
    /// Candidate and artifact roots must not overlap.
    #[error("candidate worktree and artifact directory must not overlap")]
    OverlappingDirectories,
    /// Changed path is not repository-relative and portable.
    #[error("invalid changed path: {0}")]
    ChangedPath(RepoPathError),
    /// Changed path appears more than once.
    #[error("duplicate changed path `{0}`")]
    DuplicateChangedPath(String),
    /// Cancellation identifier is unsafe.
    #[error("invalid cancellation ID: {0}")]
    CancellationId(WorkspaceValueError),
    /// Environment key cannot be passed to a child process safely.
    #[error("invalid environment key `{0}`")]
    EnvironmentKey(String),
    /// Environment value contains a NUL byte.
    #[error("environment value for `{0}` contains NUL")]
    EnvironmentValue(String),
}

/// Stable cancellation identifier, distinct from a run ID at type level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancellationId(RunId);

impl CancellationId {
    /// Returns validated identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for CancellationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(formatter)
    }
}

/// Validated identity, paths, and declared environment for one invocation.
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    run_id: RunId,
    baseline_commit: CommitId,
    evaluated_commit: CommitId,
    candidate_worktree: PathBuf,
    changed_paths: Vec<RepoPath>,
    declared_environment: BTreeMap<String, String>,
    artifact_directory: PathBuf,
    cancellation_id: CancellationId,
}

impl EvaluationContext {
    /// Validates one caller-supplied evaluator context without invoking Git or network.
    ///
    /// # Errors
    ///
    /// Rejects invalid IDs, unsafe/noncanonical directories, duplicate or
    /// escaping changed paths, and environment values a process cannot accept.
    pub fn new(spec: EvaluationContextSpec) -> Result<Self, ContextError> {
        let run_id = RunId::new(spec.run_id).map_err(ContextError::RunId)?;
        let baseline_commit =
            CommitId::new(spec.baseline_commit).map_err(ContextError::BaselineCommit)?;
        let evaluated_commit =
            CommitId::new(spec.evaluated_commit).map_err(ContextError::EvaluatedCommit)?;
        validate_directory(&spec.candidate_worktree).map_err(ContextError::CandidateWorktree)?;
        validate_directory(&spec.artifact_directory).map_err(ContextError::ArtifactDirectory)?;
        if spec
            .candidate_worktree
            .starts_with(&spec.artifact_directory)
            || spec
                .artifact_directory
                .starts_with(&spec.candidate_worktree)
        {
            return Err(ContextError::OverlappingDirectories);
        }

        let mut seen = HashSet::with_capacity(spec.changed_paths.len());
        let mut changed_paths = Vec::with_capacity(spec.changed_paths.len());
        for path in spec.changed_paths {
            let path = RepoPath::new(path).map_err(ContextError::ChangedPath)?;
            if !seen.insert(path.clone()) {
                return Err(ContextError::DuplicateChangedPath(path.as_str().to_owned()));
            }
            changed_paths.push(path);
        }

        for (key, value) in &spec.declared_environment {
            if !valid_environment_key(key) {
                return Err(ContextError::EnvironmentKey(key.clone()));
            }
            if value.contains('\0') {
                return Err(ContextError::EnvironmentValue(key.clone()));
            }
        }

        let cancellation_id =
            CancellationId(RunId::new(spec.cancellation_id).map_err(ContextError::CancellationId)?);
        Ok(Self {
            run_id,
            baseline_commit,
            evaluated_commit,
            candidate_worktree: spec.candidate_worktree,
            changed_paths,
            declared_environment: spec.declared_environment,
            artifact_directory: spec.artifact_directory,
            cancellation_id,
        })
    }

    /// Returns stable run ID.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns exact frozen baseline commit.
    #[must_use]
    pub const fn baseline_commit(&self) -> &CommitId {
        &self.baseline_commit
    }

    /// Returns exact evaluated commit.
    #[must_use]
    pub const fn evaluated_commit(&self) -> &CommitId {
        &self.evaluated_commit
    }

    /// Returns canonical isolated worktree directory.
    #[must_use]
    pub fn candidate_worktree(&self) -> &Path {
        &self.candidate_worktree
    }

    /// Returns validated changed paths in caller-declared order.
    #[must_use]
    pub fn changed_paths(&self) -> &[RepoPath] {
        &self.changed_paths
    }

    /// Returns only explicitly declared child-process environment values.
    #[must_use]
    pub const fn declared_environment(&self) -> &BTreeMap<String, String> {
        &self.declared_environment
    }

    /// Returns canonical run-owned artifact directory.
    #[must_use]
    pub fn artifact_directory(&self) -> &Path {
        &self.artifact_directory
    }

    /// Returns cancellation identifier.
    #[must_use]
    pub const fn cancellation_id(&self) -> &CancellationId {
        &self.cancellation_id
    }
}

fn validate_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err("path must be an absolute non-root directory".into());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err("path contains unresolved dot component".into());
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("cannot resolve path: {error}"))?;
    if canonical != path {
        return Err("path resolves through a symlink or alias".into());
    }
    if !fs::metadata(path)
        .map_err(|error| format!("cannot inspect path: {error}"))?
        .is_dir()
    {
        return Err("path is not a directory".into());
    }
    Ok(())
}

fn valid_environment_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
