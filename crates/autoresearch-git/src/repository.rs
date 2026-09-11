//! Read-only Git repository inspection.

use autoresearch_core::{CommitId, RepositoryInspector, RepositorySnapshot, RepositoryValueError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use thiserror::Error;

const DIAGNOSTIC_LIMIT: usize = 8 * 1024;

/// Failure to validate or operate on experiment Git state.
#[derive(Debug, Error)]
pub enum GitError {
    /// Target path does not exist.
    #[error("repository target does not exist: {}", .0.display())]
    TargetMissing(PathBuf),
    /// Target path is not a directory.
    #[error("repository target is not a directory: {}", .0.display())]
    TargetNotDirectory(PathBuf),
    /// Filesystem operation failed.
    #[error("failed to {operation} at {}: {source}", path.display())]
    Io {
        /// Stable operation description.
        operation: &'static str,
        /// Affected path.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// Target is not inside a Git worktree.
    #[error("target is not a Git worktree: {detail}")]
    NotWorktree {
        /// Bounded Git diagnostic.
        detail: String,
    },
    /// Bare repositories cannot host candidate worktrees safely.
    #[error("bare repositories cannot run experiments")]
    BareRepository,
    /// Base ref must be explicit and nonblank.
    #[error("base ref cannot be blank")]
    BlankBaseRef,
    /// Requested base ref did not resolve to a commit.
    #[error("base ref `{base_ref}` did not resolve to a commit: {detail}")]
    BaseRefNotFound {
        /// User-provided ref.
        base_ref: String,
        /// Bounded Git diagnostic.
        detail: String,
    },
    /// Current worktree has no resolvable HEAD commit.
    #[error("repository HEAD did not resolve to a commit: {detail}")]
    HeadNotFound {
        /// Bounded Git diagnostic.
        detail: String,
    },
    /// Git command failed.
    #[error("Git operation `{operation}` failed: {detail}")]
    CommandFailed {
        /// Stable operation description.
        operation: &'static str,
        /// Bounded Git diagnostic.
        detail: String,
    },
    /// Git returned output that was not UTF-8.
    #[error("Git operation `{operation}` returned non-UTF-8 output")]
    NonUtf8Output {
        /// Stable operation description.
        operation: &'static str,
    },
    /// Git returned an unexpected boolean.
    #[error("Git operation `{operation}` returned unexpected value `{value}`")]
    UnexpectedValue {
        /// Stable operation description.
        operation: &'static str,
        /// Bounded output value.
        value: String,
    },
    /// `.autoresearch/` is not ignored and could enter candidate commits.
    #[error(".autoresearch/ must be ignored by Git")]
    RunStateNotIgnored,
    /// Repository already tracks files in `.autoresearch/`.
    #[error("repository tracks run-state paths: {paths}")]
    RunStateTracked {
        /// Bounded tracked-path list.
        paths: String,
    },
    /// Caller checkout contains tracked or untracked changes.
    #[error("repository must be clean before a run: {status}")]
    DirtyRepository {
        /// Bounded porcelain status.
        status: String,
    },
    /// Domain identity returned by Git was invalid.
    #[error(transparent)]
    InvalidIdentity(#[from] RepositoryValueError),
}

/// System Git implementation of read-only repository validation.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitRepository;

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

fn git_text(root: &Path, operation: &'static str, args: &[&str]) -> Result<String, GitError> {
    let output = git_output(root, args)?;
    if !output.status.success() {
        return Err(command_error(operation, &output));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| GitError::NonUtf8Output { operation })
}

fn git_output(root: &Path, args: &[&str]) -> Result<Output, GitError> {
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

fn command_error(operation: &'static str, output: &Output) -> GitError {
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

fn bounded(value: &str) -> String {
    value.chars().take(DIAGNOSTIC_LIMIT).collect()
}
