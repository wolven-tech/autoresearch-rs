//! Lock-bound Git branch and candidate-worktree lifecycle adapter.

mod commit;
mod identity;
mod recovery;

use crate::repository::{bounded, command_error, git_output, git_text};
use crate::{GitError, RunLockGuard};
use autoresearch_core::{
    CandidateWorkspace, CandidateWorkspaceManager, CommitId, RepositorySnapshot, RunId,
    RunWorkspace,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Git lifecycle adapter whose borrow proves exclusive repository ownership.
#[derive(Debug)]
pub struct LockedGitRepository<'a> {
    snapshot: &'a RepositorySnapshot,
    _lock: &'a RunLockGuard,
    run_id: RunId,
}

impl<'a> LockedGitRepository<'a> {
    /// Returns canonical caller repository owned by this lock.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        self.snapshot.root()
    }

    /// Proves candidate still belongs to locked run and starts clean at parent.
    ///
    /// # Errors
    ///
    /// Rejects foreign/stale worktree identity, attached branch, changed HEAD,
    /// or pre-existing candidate edits before an agent command starts.
    pub fn validate_prepared_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<(), GitError> {
        self.ensure_candidate_current(run, candidate)?;
        identity::ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
        let head = resolve_commit(candidate.path(), "resolve prepared candidate HEAD", "HEAD")?;
        if &head != candidate.parent_commit() {
            return Err(GitError::CandidateTopology {
                detail: "prepared candidate HEAD differs from retained parent".into(),
            });
        }
        ensure_candidate_clean(candidate.path())
    }

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

    /// Creates or validates isolated detached baseline worktree at exact base.
    ///
    /// This deterministic path is safe to reopen after interruption. Evaluator
    /// processes never run in caller checkout. Baseline evaluation is allowed
    /// only before retained run ref advances beyond base commit.
    ///
    /// # Errors
    ///
    /// Rejects stale or foreign runs, unsafe paths, attached/dirty worktrees,
    /// and baseline HEAD mismatch.
    pub fn prepare_baseline(&self, run: &RunWorkspace) -> Result<PathBuf, GitError> {
        self.ensure_run_current(run)?;
        if run.head_commit() != run.base_commit() {
            return Err(GitError::CandidateTopology {
                detail: "baseline requires retained ref at base commit".into(),
            });
        }
        self.prepare_evaluation_worktree(run, "baseline", run.base_commit())
    }

    /// Creates or validates exact-current-commit detached worktree for an
    /// independent post-selection verification. Never moves retained ref.
    ///
    /// # Errors
    ///
    /// Rejects stale run, foreign/dirty worktree, or commit mismatch.
    pub fn prepare_verification(&self, run: &RunWorkspace) -> Result<PathBuf, GitError> {
        self.ensure_run_current(run)?;
        let name = format!("verification-{}", run.head_commit());
        self.prepare_evaluation_worktree(run, &name, run.head_commit())
    }

    fn prepare_evaluation_worktree(
        &self,
        run: &RunWorkspace,
        name: &str,
        commit: &CommitId,
    ) -> Result<PathBuf, GitError> {
        let state = self.snapshot.root().join(".autoresearch");
        let worktrees = state.join("worktrees");
        let run_worktrees = worktrees.join(run.run_id().as_str());
        for path in [&state, &worktrees, &run_worktrees] {
            ensure_real_directory(path)?;
        }
        let worktree = run_worktrees.join(name);
        match fs::symlink_metadata(&worktree) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let args = [
                    OsString::from("worktree"),
                    OsString::from("add"),
                    OsString::from("--detach"),
                    worktree.as_os_str().to_owned(),
                    OsString::from(commit.as_str()),
                ];
                git_os_text(self.snapshot.root(), "create evaluation worktree", &args)?;
                let args = [
                    OsString::from("worktree"),
                    OsString::from("lock"),
                    OsString::from("--reason"),
                    OsString::from(format!("autoresearch:{}:{name}", run.run_id())),
                    worktree.as_os_str().to_owned(),
                ];
                git_os_text(self.snapshot.root(), "lock evaluation worktree", &args)?;
            }
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(GitError::UnsafeWorktreePath {
                    path: worktree,
                    reason: "evaluation path must be a real directory",
                });
            }
            Err(error) => {
                return Err(GitError::Io {
                    operation: "inspect evaluation worktree",
                    path: worktree,
                    source: error,
                });
            }
        }
        let canonical = fs::canonicalize(&worktree).map_err(|source| GitError::Io {
            operation: "canonicalize evaluation worktree",
            path: worktree.clone(),
            source,
        })?;
        if canonical != worktree {
            return Err(GitError::UnsafeWorktreePath {
                path: worktree,
                reason: "evaluation worktree escaped deterministic path",
            });
        }
        identity::ensure_linked_worktree_identity(self.snapshot.root(), &worktree)?;
        ensure_detached(&worktree)?;
        let head = resolve_commit(&worktree, "resolve evaluation HEAD", "HEAD")?;
        if head != *commit {
            return Err(GitError::CandidateTopology {
                detail: format!("evaluation HEAD `{head}` differs from frozen commit `{commit}`"),
            });
        }
        ensure_candidate_clean(&worktree)?;
        Ok(worktree)
    }

    /// Creates and Git-locks detached candidate worktree at retained head.
    ///
    /// # Errors
    ///
    /// Rejects stale or foreign run values, existing paths, symlinked state
    /// directories, and Git worktree failures.
    pub fn prepare_candidate(
        &self,
        run: &RunWorkspace,
        index: u32,
    ) -> Result<CandidateWorkspace, GitError> {
        self.ensure_run_current(run)?;
        let worktree_root = self.snapshot.root().join(".autoresearch/worktrees");
        let candidate = CandidateWorkspace::new(run, index, &worktree_root)?;
        let state = self.snapshot.root().join(".autoresearch");
        ensure_real_directory(&state)?;
        ensure_real_directory(&worktree_root)?;
        ensure_real_directory(&worktree_root.join(run.run_id().as_str()))?;
        reject_existing_candidate(candidate.path())?;

        add_detached_worktree(self.snapshot.root(), &candidate)?;
        lock_candidate_worktree(self.snapshot.root(), &candidate)?;
        let canonical = fs::canonicalize(candidate.path()).map_err(|source| GitError::Io {
            operation: "canonicalize candidate worktree",
            path: candidate.path().to_path_buf(),
            source,
        })?;
        if canonical != candidate.path() {
            return Err(GitError::UnsafeWorktreePath {
                path: candidate.path().to_path_buf(),
                reason: "candidate worktree resolved outside deterministic path",
            });
        }
        ensure_detached(candidate.path())?;
        let head = resolve_commit(candidate.path(), "resolve candidate HEAD", "HEAD")?;
        if head != *candidate.parent_commit() {
            return Err(GitError::CandidateTopology {
                detail: format!(
                    "prepared HEAD `{head}` differs from parent `{}`",
                    candidate.parent_commit()
                ),
            });
        }
        ensure_candidate_clean(candidate.path())?;
        Ok(candidate)
    }

    /// Atomically advances retained ref to clean direct-child candidate commit.
    ///
    /// # Errors
    ///
    /// Rejects stale, foreign, attached, dirty, missing, or nonlinear
    /// candidates. Candidate worktree remains for later journal-driven cleanup.
    pub fn retain_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<RunWorkspace, GitError> {
        self.ensure_candidate_current(run, candidate)?;
        ensure_candidate_clean(candidate.path())?;
        let candidate_head = resolve_commit(candidate.path(), "resolve candidate HEAD", "HEAD")?;
        ensure_direct_child(candidate.path(), &candidate_head, run.head_commit())?;

        let output = git_output(
            self.snapshot.root(),
            &[
                "update-ref",
                run.branch_ref(),
                candidate_head.as_str(),
                run.head_commit().as_str(),
            ],
        )?;
        if !output.status.success() {
            return self.stale_or_command_error(run, "retain candidate", &output);
        }
        Ok(run.advanced_to(candidate_head))
    }

    fn ensure_candidate_current(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<(), GitError> {
        self.ensure_run_current(run)?;
        self.ensure_candidate_owned(run, candidate)
    }

    fn ensure_candidate_owned(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<(), GitError> {
        self.ensure_run_owned(run)?;
        let expected = CandidateWorkspace::new(
            run,
            candidate.index(),
            &self.snapshot.root().join(".autoresearch/worktrees"),
        )?;
        if candidate != &expected {
            return Err(GitError::ForeignCandidateWorkspace);
        }
        let canonical = fs::canonicalize(candidate.path()).map_err(|source| GitError::Io {
            operation: "canonicalize candidate worktree",
            path: candidate.path().to_path_buf(),
            source,
        })?;
        if canonical != candidate.path() {
            return Err(GitError::UnsafeWorktreePath {
                path: candidate.path().to_path_buf(),
                reason: "candidate worktree resolved outside deterministic path",
            });
        }
        ensure_detached(candidate.path())
    }

    fn ensure_run_current(&self, run: &RunWorkspace) -> Result<(), GitError> {
        self.ensure_run_owned(run)?;
        let actual = resolve_optional_ref(self.snapshot.root(), run.branch_ref())?;
        if actual.as_ref() == Some(run.head_commit()) {
            Ok(())
        } else {
            Err(GitError::StaleRunBranch {
                expected: run.head_commit().to_string(),
                actual: actual.map_or_else(|| "<missing>".into(), |commit| commit.to_string()),
            })
        }
    }

    fn ensure_run_owned(&self, run: &RunWorkspace) -> Result<(), GitError> {
        if run.run_id() != &self.run_id || run.base_commit() != self.snapshot.base_commit() {
            return Err(GitError::ForeignRunWorkspace);
        }
        if let Some(worktree) = checked_out_worktree(self.snapshot.root(), run.branch_ref())? {
            return Err(GitError::RunBranchCheckedOut {
                branch_ref: run.branch_ref().to_owned(),
                worktree,
            });
        }
        Ok(())
    }

    fn stale_or_command_error(
        &self,
        run: &RunWorkspace,
        operation: &'static str,
        output: &Output,
    ) -> Result<RunWorkspace, GitError> {
        let actual = resolve_optional_ref(self.snapshot.root(), run.branch_ref())?;
        if actual.as_ref() == Some(run.head_commit()) {
            Err(command_error(operation, output))
        } else {
            Err(GitError::StaleRunBranch {
                expected: run.head_commit().to_string(),
                actual: actual.map_or_else(|| "<missing>".into(), |commit| commit.to_string()),
            })
        }
    }
}

impl CandidateWorkspaceManager for LockedGitRepository<'_> {
    type Error = GitError;

    fn open_run(&self) -> Result<RunWorkspace, Self::Error> {
        Self::open_run(self)
    }

    fn prepare_candidate(
        &self,
        run: &RunWorkspace,
        index: u32,
    ) -> Result<CandidateWorkspace, Self::Error> {
        Self::prepare_candidate(self, run, index)
    }

    fn retain_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<RunWorkspace, Self::Error> {
        Self::retain_candidate(self, run, candidate)
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

fn ensure_real_directory(path: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(GitError::UnsafeWorktreePath {
            path: path.to_path_buf(),
            reason: "worktree parent cannot be a symlink",
        }),
        Ok(metadata) if !metadata.is_dir() => Err(GitError::UnsafeWorktreePath {
            path: path.to_path_buf(),
            reason: "worktree parent must be a directory",
        }),
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)
            .map_err(|source| GitError::Io {
                operation: "create worktree parent",
                path: path.to_path_buf(),
                source,
            }),
        Err(source) => Err(GitError::Io {
            operation: "inspect worktree parent",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn reject_existing_candidate(path: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(GitError::WorktreePathExists(path.to_path_buf())),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(GitError::Io {
            operation: "inspect candidate worktree path",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn add_detached_worktree(root: &Path, candidate: &CandidateWorkspace) -> Result<(), GitError> {
    let args = [
        OsString::from("worktree"),
        OsString::from("add"),
        OsString::from("--detach"),
        candidate.path().as_os_str().to_owned(),
        OsString::from(candidate.parent_commit().as_str()),
    ];
    git_os_text(root, "create candidate worktree", &args)?;
    Ok(())
}

fn lock_candidate_worktree(root: &Path, candidate: &CandidateWorkspace) -> Result<(), GitError> {
    let args = [
        OsString::from("worktree"),
        OsString::from("lock"),
        OsString::from("--reason"),
        OsString::from(format!("autoresearch:{}", candidate.run_id())),
        candidate.path().as_os_str().to_owned(),
    ];
    git_os_text(root, "lock candidate worktree", &args)?;
    Ok(())
}

fn ensure_detached(path: &Path) -> Result<(), GitError> {
    let output = git_output(path, &["symbolic-ref", "-q", "HEAD"])?;
    if output.status.success() {
        let branch = String::from_utf8(output.stdout).map_err(|_| GitError::NonUtf8Output {
            operation: "inspect candidate HEAD",
        })?;
        Err(GitError::CandidateNotDetached {
            branch: branch.trim().to_owned(),
        })
    } else if output.status.code() == Some(1) {
        Ok(())
    } else {
        Err(command_error("inspect candidate HEAD", &output))
    }
}

fn ensure_candidate_clean(path: &Path) -> Result<(), GitError> {
    let status = git_text(
        path,
        "inspect candidate status",
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(GitError::DirtyCandidate {
            status: bounded(&status),
        })
    }
}

fn resolve_commit(path: &Path, operation: &'static str, value: &str) -> Result<CommitId, GitError> {
    let expression = format!("{value}^{{commit}}");
    CommitId::new(git_text(
        path,
        operation,
        &["rev-parse", "--verify", &expression],
    )?)
    .map_err(Into::into)
}

fn ensure_direct_child(
    path: &Path,
    candidate: &CommitId,
    expected_parent: &CommitId,
) -> Result<(), GitError> {
    let topology = git_text(
        path,
        "inspect candidate parents",
        &["rev-list", "--parents", "-n", "1", candidate.as_str()],
    )?;
    let commits = topology.split_ascii_whitespace().collect::<Vec<_>>();
    if commits.len() == 2
        && commits[0] == candidate.as_str()
        && commits[1] == expected_parent.as_str()
    {
        Ok(())
    } else {
        Err(GitError::CandidateTopology {
            detail: bounded(&format!(
                "expected `{candidate}` with sole parent `{expected_parent}`, found `{topology}`"
            )),
        })
    }
}

fn git_os_text(
    root: &Path,
    operation: &'static str,
    args: &[OsString],
) -> Result<String, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|source| GitError::Io {
            operation: "start Git",
            path: root.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(command_error(operation, &output));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| GitError::NonUtf8Output { operation })
}
