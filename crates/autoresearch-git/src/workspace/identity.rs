//! Linked-worktree identity proof for mutation-sensitive operations.

use crate::GitError;
use crate::repository::git_text;
use autoresearch_core::CandidateWorkspace;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn ensure_candidate_repository_identity(
    repository_root: &Path,
    candidate: &CandidateWorkspace,
) -> Result<(), GitError> {
    ensure_linked_worktree_identity(repository_root, candidate.path())
}

pub(super) fn ensure_linked_worktree_identity(
    repository_root: &Path,
    worktree: &Path,
) -> Result<(), GitError> {
    let marker = worktree.join(".git");
    let metadata = fs::symlink_metadata(&marker).map_err(|source| GitError::Io {
        operation: "inspect candidate Git marker",
        path: marker.clone(),
        source,
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(GitError::CandidateRepositoryMismatch {
            detail: "candidate .git marker must be a regular file".into(),
        });
    }

    let top_level = canonical_git_path(
        worktree,
        "resolve candidate worktree root",
        "--show-toplevel",
    )?;
    if top_level != worktree {
        return Err(GitError::CandidateRepositoryMismatch {
            detail: format!(
                "worktree root `{}` differs from `{}`",
                top_level.display(),
                worktree.display()
            ),
        });
    }

    let expected_common = canonical_git_path(
        repository_root,
        "resolve repository common directory",
        "--git-common-dir",
    )?;
    let actual_common = canonical_git_path(
        worktree,
        "resolve candidate common directory",
        "--git-common-dir",
    )?;
    if actual_common != expected_common {
        return Err(GitError::CandidateRepositoryMismatch {
            detail: format!(
                "common directory `{}` differs from `{}`",
                actual_common.display(),
                expected_common.display()
            ),
        });
    }

    let admin = candidate_admin_directory(worktree, &marker)?;
    if !admin.starts_with(expected_common.join("worktrees")) {
        return Err(GitError::CandidateRepositoryMismatch {
            detail: format!(
                "administrative directory `{}` is outside linked worktrees",
                admin.display()
            ),
        });
    }
    let backlink = read_gitdir_value(&admin.join("gitdir"), "read candidate Git backlink")?;
    let backlink = canonicalize_existing(&backlink, "canonicalize candidate Git backlink")?;
    let expected_marker = canonicalize_existing(&marker, "canonicalize candidate Git marker")?;
    if backlink != expected_marker {
        return Err(GitError::CandidateRepositoryMismatch {
            detail: format!(
                "administrative backlink `{}` differs from `{}`",
                backlink.display(),
                expected_marker.display()
            ),
        });
    }
    Ok(())
}

fn canonical_git_path(
    root: &Path,
    operation: &'static str,
    selector: &str,
) -> Result<PathBuf, GitError> {
    let value = git_text(
        root,
        operation,
        &["rev-parse", "--path-format=absolute", selector],
    )?;
    canonicalize_existing(Path::new(&value), operation)
}

fn candidate_admin_directory(worktree: &Path, marker: &Path) -> Result<PathBuf, GitError> {
    let value = read_gitdir_value(marker, "read candidate Git marker")?;
    let value = if value.is_absolute() {
        value
    } else {
        worktree.join(value)
    };
    canonicalize_existing(&value, "canonicalize candidate administrative directory")
}

fn read_gitdir_value(path: &Path, operation: &'static str) -> Result<PathBuf, GitError> {
    let contents = fs::read_to_string(path).map_err(|source| GitError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    let value = contents
        .trim()
        .strip_prefix("gitdir: ")
        .unwrap_or_else(|| contents.trim());
    if value.is_empty() || contents.lines().count() != 1 {
        return Err(GitError::CandidateRepositoryMismatch {
            detail: format!("invalid Git directory pointer at `{}`", path.display()),
        });
    }
    Ok(PathBuf::from(value))
}

fn canonicalize_existing(path: &Path, operation: &'static str) -> Result<PathBuf, GitError> {
    fs::canonicalize(path).map_err(|source| GitError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })
}
