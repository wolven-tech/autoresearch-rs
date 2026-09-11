//! Repository identity values and inspection port.

use std::error::Error;
use std::fmt;
use std::path::Component;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Invalid repository-domain value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryValueError {
    /// Git object IDs must be full SHA-1 or SHA-256 hexadecimal values.
    #[error("commit ID must contain 40 or 64 hexadecimal characters")]
    InvalidCommitId,
    /// Repository roots must already be canonical absolute paths.
    #[error("repository root must be absolute")]
    RelativeRoot,
    /// Filesystem root cannot be an experiment repository.
    #[error("filesystem root cannot be an experiment repository")]
    FilesystemRoot,
    /// Repository roots cannot contain unresolved lexical components.
    #[error("repository root must not contain `.` or `..` components")]
    UnnormalizedRoot,
}

/// Exact Git commit object identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitId(String);

impl CommitId {
    /// Validates a full SHA-1 or SHA-256 object ID.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryValueError::InvalidCommitId`] for abbreviated or
    /// non-hexadecimal values.
    pub fn new(value: impl Into<String>) -> Result<Self, RepositoryValueError> {
        let value = value.into();
        if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(RepositoryValueError::InvalidCommitId)
        }
    }

    /// Returns full lowercase hexadecimal object ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Read-only identity captured from a validated clean Git worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySnapshot {
    root: PathBuf,
    base_commit: CommitId,
    head_commit: CommitId,
}

impl RepositorySnapshot {
    /// Creates a repository snapshot from validated values.
    ///
    /// # Errors
    ///
    /// Rejects relative paths and filesystem roots. Filesystem adapters remain
    /// responsible for proving canonicality and repository cleanliness.
    pub fn new(
        root: PathBuf,
        base_commit: CommitId,
        head_commit: CommitId,
    ) -> Result<Self, RepositoryValueError> {
        if !root.is_absolute() {
            return Err(RepositoryValueError::RelativeRoot);
        }
        if root.parent().is_none() {
            return Err(RepositoryValueError::FilesystemRoot);
        }
        if root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(RepositoryValueError::UnnormalizedRoot);
        }
        Ok(Self {
            root,
            base_commit,
            head_commit,
        })
    }

    /// Returns canonical repository root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns exact commit resolved from requested base ref.
    #[must_use]
    pub const fn base_commit(&self) -> &CommitId {
        &self.base_commit
    }

    /// Returns exact commit currently checked out by caller.
    #[must_use]
    pub const fn head_commit(&self) -> &CommitId {
        &self.head_commit
    }
}

/// Read-only repository validation boundary implemented by infrastructure.
pub trait RepositoryInspector {
    /// Adapter-specific diagnostic error.
    type Error: Error + Send + Sync + 'static;

    /// Validates `target` and resolves exact baseline identity from `base_ref`.
    ///
    /// # Errors
    ///
    /// Returns adapter-specific diagnostics when repository cannot safely begin
    /// an experiment.
    fn inspect(&self, target: &Path, base_ref: &str) -> Result<RepositorySnapshot, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{CommitId, RepositorySnapshot, RepositoryValueError};
    use std::path::PathBuf;

    const SHA_ONE: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn commit_identity_is_full_and_normalized() {
        let identity = CommitId::new(SHA_ONE.to_ascii_uppercase()).expect("valid identity");
        assert_eq!(identity.as_str(), SHA_ONE);
        assert_eq!(identity.to_string(), SHA_ONE);
        assert_eq!(
            CommitId::new("01234567"),
            Err(RepositoryValueError::InvalidCommitId)
        );
    }

    #[test]
    fn repository_snapshot_rejects_relative_and_root_paths() {
        let commit = CommitId::new(SHA_ONE).expect("valid identity");
        assert_eq!(
            RepositorySnapshot::new(PathBuf::from("repo"), commit.clone(), commit.clone()),
            Err(RepositoryValueError::RelativeRoot)
        );
        assert_eq!(
            RepositorySnapshot::new(PathBuf::from("/"), commit.clone(), commit),
            Err(RepositoryValueError::FilesystemRoot)
        );
    }

    #[test]
    fn repository_snapshot_rejects_lexically_unnormalized_root() {
        let commit = CommitId::new(SHA_ONE).expect("valid identity");
        assert_eq!(
            RepositorySnapshot::new(PathBuf::from("/tmp/../repository"), commit.clone(), commit),
            Err(RepositoryValueError::UnnormalizedRoot)
        );
    }
}
