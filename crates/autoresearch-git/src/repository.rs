//! Read-only Git repository inspection.

use crate::GitError;
use autoresearch_core::{CommitId, RepoPath, RepositoryInspector, RepositorySnapshot};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DIAGNOSTIC_LIMIT: usize = 8 * 1024;

/// System Git implementation of read-only repository validation.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitRepository;

impl GitRepository {
    /// Reads one ordinary file from frozen base commit, never caller checkout.
    /// Missing optional path returns `None`; symlink and oversized blobs fail.
    ///
    /// # Errors
    ///
    /// Rejects malformed Git metadata, non-file tree entries, and oversized
    /// content before asking Git to return blob bytes.
    pub fn read_blob_at_commit(
        &self,
        snapshot: &RepositorySnapshot,
        path: &RepoPath,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, GitError> {
        let commit = snapshot.base_commit().as_str();
        let tree = git_output(
            snapshot.root(),
            &["ls-tree", "-z", "--full-tree", commit, "--", path.as_str()],
        )?;
        if !tree.status.success() {
            return Err(command_error("read frozen tree entry", &tree));
        }
        if tree.stdout.is_empty() {
            return Ok(None);
        }
        let mut records = tree
            .stdout
            .split(|byte| *byte == 0)
            .filter(|r| !r.is_empty());
        let record = records.next().ok_or_else(|| GitError::UnsafeSourceBlob {
            path: path.as_str().into(),
            reason: "tree entry missing",
        })?;
        let (metadata, actual_path) =
            record
                .split_once_byte(b'\t')
                .ok_or_else(|| GitError::UnsafeSourceBlob {
                    path: path.as_str().into(),
                    reason: "malformed tree entry",
                })?;
        if records.next().is_some() || actual_path != path.as_str().as_bytes() {
            return Err(GitError::UnsafeSourceBlob {
                path: path.as_str().into(),
                reason: "tree entry did not match exact path",
            });
        }
        let metadata = std::str::from_utf8(metadata).map_err(|_| GitError::UnsafeSourceBlob {
            path: path.as_str().into(),
            reason: "tree metadata is not UTF-8",
        })?;
        let mut fields = metadata.split_whitespace();
        let mode = fields.next();
        let kind = fields.next();
        let object = fields.next();
        if !matches!(mode, Some("100644" | "100755"))
            || kind != Some("blob")
            || fields.next().is_some()
        {
            return Err(GitError::UnsafeSourceBlob {
                path: path.as_str().into(),
                reason: "source is not an ordinary tracked file",
            });
        }
        let object = object.ok_or_else(|| GitError::UnsafeSourceBlob {
            path: path.as_str().into(),
            reason: "tree entry has no object ID",
        })?;
        let size = git_text(
            snapshot.root(),
            "measure frozen source blob",
            &["cat-file", "-s", object],
        )?
        .parse::<u64>()
        .map_err(|_| GitError::UnsafeSourceBlob {
            path: path.as_str().into(),
            reason: "blob size is invalid",
        })?;
        if size > max_bytes {
            return Err(GitError::UnsafeSourceBlob {
                path: path.as_str().into(),
                reason: "blob exceeds byte limit",
            });
        }
        let blob = git_output(snapshot.root(), &["cat-file", "blob", object])?;
        if !blob.status.success() {
            return Err(command_error("read frozen source blob", &blob));
        }
        if u64::try_from(blob.stdout.len()).ok() != Some(size) {
            return Err(GitError::UnsafeSourceBlob {
                path: path.as_str().into(),
                reason: "blob changed size during read",
            });
        }
        Ok(Some(blob.stdout))
    }
}

trait SplitOnceByte {
    fn split_once_byte(&self, separator: u8) -> Option<(&[u8], &[u8])>;
}

impl SplitOnceByte for [u8] {
    fn split_once_byte(&self, separator: u8) -> Option<(&[u8], &[u8])> {
        let index = self.iter().position(|byte| *byte == separator)?;
        Some((&self[..index], &self[index + 1..]))
    }
}

impl RepositoryInspector for GitRepository {
    type Error = GitError;

    fn inspect(&self, target: &Path, base_ref: &str) -> Result<RepositorySnapshot, Self::Error> {
        inspect(target, base_ref)
    }
}

fn inspect(target: &Path, base_ref: &str) -> Result<RepositorySnapshot, GitError> {
    if base_ref.trim().is_empty() {
        return Err(GitError::BlankBaseRef);
    }
    let target = canonical_target(target)?;
    let bare = git_boolean(
        &target,
        "detect bare repository",
        &["rev-parse", "--is-bare-repository"],
    )
    .map_err(|error| GitError::NotWorktree {
        detail: error.to_string(),
    })?;
    if bare {
        return Err(GitError::BareRepository);
    }
    let inside = git_boolean(
        &target,
        "detect worktree",
        &["rev-parse", "--is-inside-work-tree"],
    )
    .map_err(|error| GitError::NotWorktree {
        detail: error.to_string(),
    })?;
    if !inside {
        return Err(GitError::NotWorktree {
            detail: "Git reported target outside worktree".into(),
        });
    }

    let root = PathBuf::from(git_text(
        &target,
        "resolve repository root",
        &["rev-parse", "--show-toplevel"],
    )?);
    let root = fs::canonicalize(&root).map_err(|source| GitError::Io {
        operation: "canonicalize repository root",
        path: root,
        source,
    })?;
    ensure_run_state_safe(&root)?;
    ensure_clean(&root)?;

    let base_argument = format!("{base_ref}^{{commit}}");
    let base_commit = git_text(
        &root,
        "resolve base ref",
        &["rev-parse", "--verify", &base_argument],
    )
    .map_err(|error| GitError::BaseRefNotFound {
        base_ref: base_ref.to_owned(),
        detail: error.to_string(),
    })?;
    let head_commit = git_text(
        &root,
        "resolve HEAD",
        &["rev-parse", "--verify", "HEAD^{commit}"],
    )
    .map_err(|error| GitError::HeadNotFound {
        detail: error.to_string(),
    })?;

    RepositorySnapshot::new(
        root,
        CommitId::new(base_commit)?,
        CommitId::new(head_commit)?,
    )
    .map_err(Into::into)
}

fn canonical_target(target: &Path) -> Result<PathBuf, GitError> {
    let target = fs::canonicalize(target).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            GitError::TargetMissing(target.to_path_buf())
        } else {
            GitError::Io {
                operation: "canonicalize repository target",
                path: target.to_path_buf(),
                source,
            }
        }
    })?;
    if target.is_dir() {
        Ok(target)
    } else {
        Err(GitError::TargetNotDirectory(target))
    }
}

fn ensure_run_state_safe(root: &Path) -> Result<(), GitError> {
    let tracked = git_text(
        root,
        "list tracked run state",
        &["ls-files", "--", ".autoresearch"],
    )?;
    if !tracked.is_empty() {
        return Err(GitError::RunStateTracked {
            paths: bounded(&tracked),
        });
    }

    let state_path = root.join(".autoresearch");
    match fs::symlink_metadata(&state_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(GitError::UnsafeRunState {
                path: state_path,
                reason: "run-state directory cannot be a symlink",
            });
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(GitError::UnsafeRunState {
                path: state_path,
                reason: "run-state path must be a directory",
            });
        }
        Ok(_) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(GitError::Io {
                operation: "inspect run-state path",
                path: state_path,
                source,
            });
        }
    }

    let output = git_output(root, &["check-ignore", "-q", ".autoresearch/probe"])?;
    if output.status.success() {
        Ok(())
    } else if output.status.code() == Some(1) {
        Err(GitError::RunStateNotIgnored)
    } else {
        Err(command_error("check ignored run state", &output))
    }
}

fn ensure_clean(root: &Path) -> Result<(), GitError> {
    let status = git_text(
        root,
        "inspect repository status",
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(GitError::DirtyRepository {
            status: bounded(&status),
        })
    }
}

fn git_boolean(root: &Path, operation: &'static str, args: &[&str]) -> Result<bool, GitError> {
    match git_text(root, operation, args)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(GitError::UnexpectedValue {
            operation,
            value: bounded(value),
        }),
    }
}

pub(crate) fn git_text(
    root: &Path,
    operation: &'static str,
    args: &[&str],
) -> Result<String, GitError> {
    let output = git_output(root, args)?;
    if !output.status.success() {
        return Err(command_error(operation, &output));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| GitError::NonUtf8Output { operation })
}

pub(crate) fn git_output(root: &Path, args: &[&str]) -> Result<Output, GitError> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|source| GitError::Io {
            operation: "start Git",
            path: root.to_path_buf(),
            source,
        })
}

pub(crate) fn command_error(operation: &'static str, output: &Output) -> GitError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    GitError::CommandFailed {
        operation,
        detail: bounded(detail),
    }
}

pub(crate) fn bounded(value: &str) -> String {
    value.chars().take(DIAGNOSTIC_LIMIT).collect()
}
