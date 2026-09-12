//! Pure repository mutation boundaries and candidate commit evidence.

use crate::{CandidateWorkspace, CommitId, RunWorkspace};
use serde::Serialize;
use std::error::Error;
use thiserror::Error;

/// Invalid repository-relative path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unsafe repository path `{path}`: {reason}")]
pub struct RepoPathError {
    path: String,
    reason: &'static str,
}

impl RepoPathError {
    /// Returns rejected path text.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns stable rejection reason.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}

/// Safe forward-slash repository-relative path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    /// Validates one portable repository-relative path.
    ///
    /// # Errors
    ///
    /// Rejects blank, absolute, backslash-bearing, control-character-bearing,
    /// empty-segment, and dot-segment paths.
    pub fn new(path: impl Into<String>) -> Result<Self, RepoPathError> {
        let path = path.into();
        let reason = if path.is_empty() {
            Some("path cannot be empty")
        } else if path.starts_with('/') {
            Some("absolute paths are forbidden")
        } else if path.contains('\\') {
            Some("use forward slashes for portable identities")
        } else if path.contains('\0') {
            Some("NUL bytes are forbidden")
        } else if path.bytes().any(|byte| byte.is_ascii_control()) {
            Some("ASCII control characters are forbidden")
        } else if path.split('/').any(str::is_empty) {
            Some("empty path segments are forbidden")
        } else if path.split('/').any(|segment| matches!(segment, "." | "..")) {
            Some("dot and parent segments are forbidden")
        } else {
            None
        };

        if let Some(reason) = reason {
            Err(RepoPathError { path, reason })
        } else {
            Ok(Self(path))
        }
    }

    /// Returns normalized relative representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_within(&self, parent: &Self) -> bool {
        self == parent
            || self
                .0
                .strip_prefix(&parent.0)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

/// Invalid mutation-boundary definition.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MutationBoundaryError {
    /// At least one mutation root must be declared.
    #[error("mutation boundary requires at least one mutable path")]
    EmptyMutablePaths,
    /// Mutable root cannot itself be protected.
    #[error("mutable path `{mutable}` is inside protected path `{protected}`")]
    MutableProtected {
        /// Rejected mutable root.
        mutable: String,
        /// Protecting root.
        protected: String,
    },
}

/// One changed path violates frozen mutation scope.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContainmentViolation {
    /// Changed path falls within protected root.
    #[error("changed path `{path}` is protected by `{protected}`")]
    ProtectedPath {
        /// Rejected changed path.
        path: String,
        /// Matching protected root.
        protected: String,
    },
    /// Changed path does not fall within any mutable root.
    #[error("changed path `{path}` is outside declared mutable paths")]
    OutsideMutable {
        /// Rejected changed path.
        path: String,
    },
}

/// Normalized mutable roots with protected-path carve-outs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MutationBoundary {
    mutable_paths: Vec<RepoPath>,
    protected_paths: Vec<RepoPath>,
}

impl MutationBoundary {
    /// Normalizes and validates frozen mutation roots.
    ///
    /// # Errors
    ///
    /// Rejects an empty mutable set or a mutable root already inside a
    /// protected root. Protected descendants of broad mutable roots remain
    /// valid carve-outs and are checked before allow rules.
    pub fn new(
        mut mutable_paths: Vec<RepoPath>,
        mut protected_paths: Vec<RepoPath>,
    ) -> Result<Self, MutationBoundaryError> {
        mutable_paths.sort();
        mutable_paths.dedup();
        protected_paths.sort();
        protected_paths.dedup();
        if mutable_paths.is_empty() {
            return Err(MutationBoundaryError::EmptyMutablePaths);
        }
        for mutable in &mutable_paths {
            if let Some(protected) = protected_paths
                .iter()
                .find(|protected| mutable.is_within(protected))
            {
                return Err(MutationBoundaryError::MutableProtected {
                    mutable: mutable.as_str().to_owned(),
                    protected: protected.as_str().to_owned(),
                });
            }
        }
        Ok(Self {
            mutable_paths,
            protected_paths,
        })
    }

    /// Returns normalized mutation roots.
    #[must_use]
    pub fn mutable_paths(&self) -> &[RepoPath] {
        &self.mutable_paths
    }

    /// Returns normalized immutable roots.
    #[must_use]
    pub fn protected_paths(&self) -> &[RepoPath] {
        &self.protected_paths
    }

    /// Proves one changed path is mutable and not protected.
    ///
    /// # Errors
    ///
    /// Returns a typed protected-path or outside-scope violation.
    pub fn validate(&self, path: &RepoPath) -> Result<(), ContainmentViolation> {
        if let Some(protected) = self
            .protected_paths
            .iter()
            .find(|protected| path.is_within(protected))
        {
            return Err(ContainmentViolation::ProtectedPath {
                path: path.as_str().to_owned(),
                protected: protected.as_str().to_owned(),
            });
        }
        if self
            .mutable_paths
            .iter()
            .any(|mutable| path.is_within(mutable))
        {
            Ok(())
        } else {
            Err(ContainmentViolation::OutsideMutable {
                path: path.as_str().to_owned(),
            })
        }
    }
}

/// Invalid candidate commit evidence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CandidateCommitError {
    /// A candidate cannot be comparable without a diff.
    #[error("candidate commit requires at least one changed path")]
    NoChangedPaths,
    /// Changed path violates frozen scope.
    #[error(transparent)]
    Containment(#[from] ContainmentViolation),
}

/// Exact candidate revision plus canonical contained diff paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCommit {
    commit_id: CommitId,
    changed_paths: Vec<RepoPath>,
}

impl CandidateCommit {
    /// Creates contained commit evidence from adapter-observed paths.
    ///
    /// # Errors
    ///
    /// Rejects empty or boundary-violating path sets.
    pub fn new(
        commit_id: CommitId,
        mut changed_paths: Vec<RepoPath>,
        boundary: &MutationBoundary,
    ) -> Result<Self, CandidateCommitError> {
        changed_paths.sort();
        changed_paths.dedup();
        if changed_paths.is_empty() {
            return Err(CandidateCommitError::NoChangedPaths);
        }
        for path in &changed_paths {
            boundary.validate(path)?;
        }
        Ok(Self {
            commit_id,
            changed_paths,
        })
    }

    /// Returns exact candidate commit identity.
    #[must_use]
    pub const fn commit_id(&self) -> &CommitId {
        &self.commit_id
    }

    /// Returns sorted unique changed paths.
    #[must_use]
    pub fn changed_paths(&self) -> &[RepoPath] {
        &self.changed_paths
    }
}

/// Port that turns one isolated mutation into one contained Git commit.
pub trait CandidateCommitter {
    /// Adapter-specific diagnostic error.
    type Error: Error + Send + Sync + 'static;

    /// Validates and commits all candidate changes under frozen boundary.
    ///
    /// # Errors
    ///
    /// Returns adapter diagnostics without advancing retained run branch.
    fn commit_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        boundary: &MutationBoundary,
    ) -> Result<CandidateCommit, Self::Error>;
}
