//! Lock-bound Git branch and candidate-worktree lifecycle adapter.

use crate::repository::{command_error, git_output, git_text};
use crate::{GitError, RunLockGuard};
use autoresearch_core::{CommitId, RepositorySnapshot, RunId, RunWorkspace};
use std::path::{Path, PathBuf};

/// Git lifecycle adapter whose borrow proves exclusive repository ownership.
#[derive(Debug)]
pub struct LockedGitRepository<'a> {
    snapshot: &'a RepositorySnapshot,
    _lock: &'a RunLockGuard,
    run_id: RunId,
}

impl<'a> LockedGitRepository<'a> {
    /// Binds validated repository snapshot to matching live lock.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::LockRepositoryMismatch`] when lock path belongs to
    /// another root, or [`GitError::InvalidRunId`] for invalid owner metadata.
    pub fn new(snapshot: &'a RepositorySnapshot, lock: &'a RunLockGuard) -> Result<Self, GitError> {
        let expected_lock = snapshot.root().join(".autoresearch/run.lock");
        if lock.path() != expected_lock {
            return Err(GitError::LockRepositoryMismatch);
        }
        let run_id = RunId::new(lock.owner().run_id()).map_err(|_| GitError::InvalidRunId)?;
        Ok(Self {
            snapshot,
            _lock: lock,
            run_id,
        })
    }

    /// Creates retained run ref or resumes its descendant current-best commit.
    ///
    /// # Errors
    ///
    /// Rejects Git failures, unrelated existing refs, or any worktree that has
    /// retained ref checked out.
    pub fn open_run(&self) -> Result<RunWorkspace, GitError> {
        let provisional = RunWorkspace::new(
            self.run_id.clone(),
            self.snapshot.base_commit().clone(),
            self.snapshot.base_commit().clone(),
        );
        let branch_ref = provisional.branch_ref();
        if let Some(worktree) = checked_out_worktree(self.snapshot.root(), branch_ref)? {
            return Err(GitError::RunBranchCheckedOut {
                branch_ref: branch_ref.to_owned(),
                worktree,
            });
        }

        let head_commit =
            if let Some(existing) = resolve_optional_ref(self.snapshot.root(), branch_ref)? {
                ensure_descendant(
                    self.snapshot.root(),
                    branch_ref,
                    self.snapshot.base_commit(),
                    &existing,
                )?;
                existing
            } else {
                create_run_ref(
                    self.snapshot.root(),
                    branch_ref,
                    self.snapshot.base_commit(),
                )?;
                self.snapshot.base_commit().clone()
            };
        Ok(RunWorkspace::new(
            self.run_id.clone(),
            self.snapshot.base_commit().clone(),
            head_commit,
        ))
    }
}

fn resolve_optional_ref(root: &Path, branch_ref: &str) -> Result<Option<CommitId>, GitError> {
    let commit_ref = format!("{branch_ref}^{{commit}}");
    let output = git_output(root, &["rev-parse", "--verify", "--quiet", &commit_ref])?;
    if output.status.success() {
        let value = String::from_utf8(output.stdout).map_err(|_| GitError::NonUtf8Output {
            operation: "resolve run branch",
        })?;
        Ok(Some(CommitId::new(value.trim().to_owned())?))
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        Err(command_error("resolve run branch", &output))
    }
}

fn create_run_ref(root: &Path, branch_ref: &str, base: &CommitId) -> Result<(), GitError> {
    let zero = "0".repeat(base.as_str().len());
    git_text(
        root,
        "create run branch",
        &[
            "update-ref",
            "--create-reflog",
            branch_ref,
            base.as_str(),
            &zero,
        ],
    )?;
    Ok(())
}

fn ensure_descendant(
    root: &Path,
    branch_ref: &str,
    base: &CommitId,
    head: &CommitId,
) -> Result<(), GitError> {
    let output = git_output(
        root,
        &["merge-base", "--is-ancestor", base.as_str(), head.as_str()],
    )?;
    if output.status.success() {
        Ok(())
    } else if output.status.code() == Some(1) {
        Err(GitError::RunBranchDiverged {
            branch_ref: branch_ref.to_owned(),
            base_commit: base.to_string(),
            head_commit: head.to_string(),
        })
    } else {
        Err(command_error("validate run branch ancestry", &output))
    }
}

fn checked_out_worktree(root: &Path, branch_ref: &str) -> Result<Option<PathBuf>, GitError> {
    let listing = git_text(
        root,
        "list repository worktrees",
        &["worktree", "list", "--porcelain"],
    )?;
    let mut worktree = None;
    for line in listing.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            worktree = Some(PathBuf::from(path));
        } else if line.strip_prefix("branch ") == Some(branch_ref) {
            return Ok(worktree);
        } else if line.is_empty() {
            worktree = None;
        }
    }
    Ok(None)
}
