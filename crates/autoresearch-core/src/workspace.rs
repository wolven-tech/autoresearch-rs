//! Pure values and port for isolated candidate workspace lifecycle.

use crate::CommitId;
use serde::{Deserialize, Deserializer, Serialize};
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Invalid workspace lifecycle value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkspaceValueError {
    /// Run IDs become both path and Git-ref components.
    #[error("run ID must be 1-128 ASCII letters, digits, dashes, or underscores")]
    InvalidRunId,
    /// Candidate numbering starts at one to match journal semantics.
    #[error("candidate index must be greater than zero")]
    ZeroCandidateIndex,
    /// Worktree roots must be absolute.
    #[error("worktree root must be absolute")]
    RelativeWorktreeRoot,
    /// Filesystem root is never a valid worktree root.
    #[error("filesystem root cannot be a worktree root")]
    FilesystemRoot,
    /// Worktree roots cannot contain unresolved lexical components.
    #[error("worktree root must not contain `.` or `..` components")]
    UnnormalizedWorktreeRoot,
}

/// Stable identifier safe as one filesystem and Git-ref component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Validates one run identifier.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceValueError::InvalidRunId`] for blank, long, or
    /// punctuation-bearing input that could escape a path or invalidate a ref.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceValueError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            Err(WorkspaceValueError::InvalidRunId)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns validated run identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for RunId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Retained current-best branch for one experiment run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunWorkspace {
    run_id: RunId,
    branch_ref: String,
    base_commit: CommitId,
    head_commit: CommitId,
}

impl RunWorkspace {
    /// Creates retained run state with canonical branch naming.
    #[must_use]
    pub fn new(run_id: RunId, base_commit: CommitId, head_commit: CommitId) -> Self {
        let branch_ref = format!("refs/heads/autoresearch/{run_id}");
        Self {
            run_id,
            branch_ref,
            base_commit,
            head_commit,
        }
    }

    /// Returns run identifier.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns full retained Git ref.
    #[must_use]
    pub fn branch_ref(&self) -> &str {
        &self.branch_ref
    }

    /// Returns immutable starting commit.
    #[must_use]
    pub const fn base_commit(&self) -> &CommitId {
        &self.base_commit
    }

    /// Returns current-best commit.
    #[must_use]
    pub const fn head_commit(&self) -> &CommitId {
        &self.head_commit
    }

    /// Returns same run advanced to a verified retained commit.
    #[must_use]
    pub fn advanced_to(&self, head_commit: CommitId) -> Self {
        Self::new(self.run_id.clone(), self.base_commit.clone(), head_commit)
    }
}

/// Detached worktree prepared from one retained run head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateWorkspace {
    run_id: RunId,
    index: u32,
    worktree_id: String,
    path: PathBuf,
    parent_commit: CommitId,
}

impl CandidateWorkspace {
    /// Derives deterministic candidate identity and path below worktree root.
    ///
    /// # Errors
    ///
    /// Rejects zero indices and roots that are relative, filesystem root, or
    /// lexically unresolved. Infrastructure must additionally reject symlinks.
    pub fn new(
        run: &RunWorkspace,
        index: u32,
        worktree_root: &Path,
    ) -> Result<Self, WorkspaceValueError> {
        validate_worktree_root(worktree_root)?;
        if index == 0 {
            return Err(WorkspaceValueError::ZeroCandidateIndex);
        }
        let worktree_id = format!("candidate-{index:06}");
        let path = worktree_root.join(run.run_id.as_str()).join(&worktree_id);
        Ok(Self {
            run_id: run.run_id.clone(),
            index,
            worktree_id,
            path,
            parent_commit: run.head_commit.clone(),
        })
    }

    /// Returns owning run identifier.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns one-based candidate number.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns relocation-independent worktree identifier.
    #[must_use]
    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
    }

    /// Returns absolute candidate worktree path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns current-best commit from which candidate started.
    #[must_use]
    pub const fn parent_commit(&self) -> &CommitId {
        &self.parent_commit
    }
}

/// Port required by application layer for candidate Git isolation.
pub trait CandidateWorkspaceManager {
    /// Adapter-specific diagnostic error.
    type Error: Error + Send + Sync + 'static;

    /// Creates or resumes retained run branch.
    ///
    /// # Errors
    ///
    /// Returns adapter diagnostics when run identity cannot be opened safely.
    fn open_run(&self) -> Result<RunWorkspace, Self::Error>;

    /// Creates detached candidate worktree from current retained head.
    ///
    /// # Errors
    ///
    /// Returns adapter diagnostics for stale run state or unsafe worktree path.
    fn prepare_candidate(
        &self,
        run: &RunWorkspace,
        index: u32,
    ) -> Result<CandidateWorkspace, Self::Error>;

    /// Atomically advances retained branch to verified candidate commit.
    ///
    /// # Errors
    ///
    /// Returns adapter diagnostics when candidate is dirty, foreign, nonlinear,
    /// or based on stale run state.
    fn retain_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<RunWorkspace, Self::Error>;
}

fn validate_worktree_root(path: &Path) -> Result<(), WorkspaceValueError> {
    if !path.is_absolute() {
        return Err(WorkspaceValueError::RelativeWorktreeRoot);
    }
    if path.parent().is_none() {
        return Err(WorkspaceValueError::FilesystemRoot);
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(WorkspaceValueError::UnnormalizedWorktreeRoot);
    }
    Ok(())
}
