//! Journal-driven candidate reconciliation and exact worktree cleanup.

use super::identity::ensure_candidate_repository_identity;
use super::{
    LockedGitRepository, ensure_candidate_clean, ensure_detached, ensure_direct_child, git_os_text,
    lock_candidate_worktree, resolve_commit, resolve_optional_ref,
};
use crate::GitError;
use crate::repository::{bounded, command_error, git_output};
use autoresearch_core::{
    CandidateFinalization, CandidateRecovery, CandidateRecoveryManager, CandidateRecoveryRequest,
    CandidateWorkspace, CommitId, Disposition, RecoveredCandidateState, RunView, RunWorkspace,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

impl LockedGitRepository<'_> {
    /// Reconciles replayed journal state with exact candidate Git effects.
    ///
    /// # Errors
    ///
    /// Rejects invalid journal values, foreign or dirty candidate state,
    /// retained-ref conflicts, registration mismatches, and unsafe cleanup.
    pub fn recover_candidate(&self, view: &RunView) -> Result<Option<CandidateRecovery>, GitError> {
        let root = self.snapshot.root().join(".autoresearch/worktrees");
        let Some(request) = CandidateRecoveryRequest::from_run_view(view, &root)? else {
            return Ok(None);
        };
        match request {
            CandidateRecoveryRequest::Evaluate { run, candidate } => {
                self.recover_for_evaluation(&run, &candidate).map(Some)
            }
            CandidateRecoveryRequest::Finalize {
                run,
                candidate,
                candidate_commit,
                decision,
            } => self
                .recover_finalization(&run, &candidate, &candidate_commit, decision.disposition)
                .map(Some),
        }
    }

    fn recover_for_evaluation(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<CandidateRecovery, GitError> {
        self.ensure_run_current(run)?;
        let candidate = match self.candidate_presence(run, candidate)? {
            CandidatePresence::Missing => self.prepare_candidate(run, candidate.index())?,
            CandidatePresence::Present { locked, .. } => {
                self.ensure_candidate_current(run, candidate)?;
                ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
                if !locked {
                    lock_candidate_worktree(self.snapshot.root(), candidate)?;
                }
                candidate.clone()
            }
        };
        ensure_candidate_clean(candidate.path())?;
        let commit = resolve_commit(candidate.path(), "resolve recovered candidate HEAD", "HEAD")?;
        let state = if commit == *candidate.parent_commit() {
            RecoveredCandidateState::Prepared
        } else {
            ensure_direct_child(candidate.path(), &commit, candidate.parent_commit())?;
            RecoveredCandidateState::Committed { commit }
        };
        Ok(CandidateRecovery::EvaluationReady {
            run: run.clone(),
            candidate,
            state,
        })
    }

    fn recover_finalization(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        candidate_commit: &CommitId,
        disposition: Disposition,
    ) -> Result<CandidateRecovery, GitError> {
        self.ensure_run_owned(run)?;
        let actual =
            resolve_optional_ref(self.snapshot.root(), run.branch_ref())?.ok_or_else(|| {
                GitError::RecoveryStateConflict {
                    detail: format!("retained ref `{}` is missing", run.branch_ref()),
                }
            })?;
        let presence = self.candidate_presence(run, candidate)?;

        match disposition {
            Disposition::Keep => {
                self.finalize_keep(run, candidate, candidate_commit, actual, presence)
            }
            Disposition::Discard => {
                self.finalize_discard(run, candidate, candidate_commit, &actual, presence)
            }
        }
    }

    fn finalize_keep(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        candidate_commit: &CommitId,
        actual: CommitId,
        presence: CandidatePresence,
    ) -> Result<CandidateRecovery, GitError> {
        let (advanced, commit) = match presence {
            CandidatePresence::Present { head, .. } => {
                self.ensure_candidate_owned(run, candidate)?;
                ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
                ensure_candidate_clean(candidate.path())?;
                ensure_direct_child(candidate.path(), &head, candidate.parent_commit())?;
                if &head != candidate_commit {
                    return Err(commit_conflict(candidate_commit, &head));
                }
                let advanced = if actual == *run.head_commit() {
                    self.retain_candidate(run, candidate)?
                } else if actual == head {
                    run.advanced_to(head.clone())
                } else {
                    return Err(ref_conflict(run, &actual, Some(&head)));
                };
                self.remove_candidate(run, candidate)?;
                (advanced, head)
            }
            CandidatePresence::Missing => {
                if actual == *run.head_commit() {
                    return Err(GitError::RecoveryStateConflict {
                        detail: "keep decision has neither candidate worktree nor advanced ref"
                            .into(),
                    });
                }
                if &actual != candidate_commit {
                    return Err(commit_conflict(candidate_commit, &actual));
                }
                ensure_direct_child(self.snapshot.root(), candidate_commit, run.head_commit())?;
                (run.advanced_to(actual.clone()), actual)
            }
        };
        Ok(CandidateRecovery::Finalized {
            run: advanced,
            outcome: CandidateFinalization::Kept {
                commit: commit.to_string(),
            },
        })
    }

    fn finalize_discard(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        candidate_commit: &CommitId,
        actual: &CommitId,
        presence: CandidatePresence,
    ) -> Result<CandidateRecovery, GitError> {
        if actual != run.head_commit() {
            return Err(ref_conflict(run, actual, None));
        }
        if let CandidatePresence::Present { head, .. } = presence {
            self.ensure_candidate_owned(run, candidate)?;
            ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
            ensure_candidate_clean(candidate.path())?;
            if &head != candidate_commit {
                return Err(commit_conflict(candidate_commit, &head));
            }
            ensure_direct_child(
                candidate.path(),
                candidate_commit,
                candidate.parent_commit(),
            )?;
            self.remove_candidate(run, candidate)?;
        }
        Ok(CandidateRecovery::Finalized {
            run: run.clone(),
            outcome: CandidateFinalization::Discarded,
        })
    }

    fn candidate_presence(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<CandidatePresence, GitError> {
        let metadata = fs::symlink_metadata(candidate.path());
        let records = registered_worktrees(self.snapshot.root())?;
        let matching = records
            .into_iter()
            .filter(|record| record.path == candidate.path())
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(registration_mismatch(
                candidate,
                "candidate path appears more than once in Git worktree registry",
            ));
        }
        let record = matching.into_iter().next();
        match (metadata, record) {
            (Err(source), None) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok(CandidatePresence::Missing)
            }
            (Err(source), None) => Err(GitError::Io {
                operation: "inspect candidate recovery path",
                path: candidate.path().to_path_buf(),
                source,
            }),
            (Ok(_), None) => Err(registration_mismatch(
                candidate,
                "path exists but Git has no linked-worktree record",
            )),
            (Err(source), Some(_)) if source.kind() == std::io::ErrorKind::NotFound => Err(
                registration_mismatch(candidate, "Git record exists but path is missing"),
            ),
            (Err(source), Some(_)) => Err(GitError::Io {
                operation: "inspect registered candidate recovery path",
                path: candidate.path().to_path_buf(),
                source,
            }),
            (Ok(metadata), Some(record)) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(registration_mismatch(
                        candidate,
                        "registered candidate path is not a real directory",
                    ));
                }
                let canonical =
                    fs::canonicalize(candidate.path()).map_err(|source| GitError::Io {
                        operation: "canonicalize candidate recovery path",
                        path: candidate.path().to_path_buf(),
                        source,
                    })?;
                if canonical != candidate.path() {
                    return Err(registration_mismatch(
                        candidate,
                        "registered candidate resolves outside deterministic path",
                    ));
                }
                if !record.detached {
                    return Err(GitError::CandidateNotDetached {
                        branch: record.branch.unwrap_or_else(|| "<unknown>".into()),
                    });
                }
                let expected_reason = format!("autoresearch:{}", run.run_id());
                let locked = match record.lock_reason {
                    None => false,
                    Some(reason) if reason == expected_reason => true,
                    Some(reason) => {
                        return Err(GitError::ForeignWorktreeLock {
                            path: candidate.path().to_path_buf(),
                            reason,
                        });
                    }
                };
                ensure_detached(candidate.path())?;
                ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
                let head = resolve_commit(
                    candidate.path(),
                    "resolve registered candidate HEAD",
                    "HEAD",
                )?;
                if head != record.head {
                    return Err(registration_mismatch(
                        candidate,
                        "Git registry HEAD differs from candidate HEAD",
                    ));
                }
                Ok(CandidatePresence::Present { head, locked })
            }
        }
    }

    fn remove_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
    ) -> Result<(), GitError> {
        let CandidatePresence::Present { locked, .. } = self.candidate_presence(run, candidate)?
        else {
            return Ok(());
        };
        ensure_candidate_clean(candidate.path())?;
        if locked {
            let args = [
                OsString::from("worktree"),
                OsString::from("unlock"),
                candidate.path().as_os_str().to_owned(),
            ];
            git_os_text(self.snapshot.root(), "unlock candidate worktree", &args)?;
        }
        let args = [
            OsString::from("worktree"),
            OsString::from("remove"),
            candidate.path().as_os_str().to_owned(),
        ];
        git_os_text(self.snapshot.root(), "remove candidate worktree", &args)?;
        match self.candidate_presence(run, candidate)? {
            CandidatePresence::Missing => Ok(()),
            CandidatePresence::Present { .. } => Err(registration_mismatch(
                candidate,
                "candidate remained registered after Git removal",
            )),
        }
    }
}

impl CandidateRecoveryManager for LockedGitRepository<'_> {
    type Error = GitError;

    fn recover_candidate(&self, view: &RunView) -> Result<Option<CandidateRecovery>, Self::Error> {
        Self::recover_candidate(self, view)
    }
}

#[derive(Debug)]
enum CandidatePresence {
    Missing,
    Present { head: CommitId, locked: bool },
}

#[derive(Debug)]
struct RegisteredWorktree {
    path: PathBuf,
    head: CommitId,
    branch: Option<String>,
    detached: bool,
    lock_reason: Option<String>,
}

#[derive(Default)]
struct WorktreeRecordBuilder {
    path: Option<PathBuf>,
    head: Option<CommitId>,
    branch: Option<String>,
    detached: bool,
    lock_reason: Option<String>,
}

impl WorktreeRecordBuilder {
    fn finish(self) -> Result<Option<RegisteredWorktree>, GitError> {
        let Some(path) = self.path else {
            return Ok(None);
        };
        let head = self.head.ok_or_else(|| GitError::RecoveryStateConflict {
            detail: format!("worktree registry omitted HEAD for `{}`", path.display()),
        })?;
        Ok(Some(RegisteredWorktree {
            path,
            head,
            branch: self.branch,
            detached: self.detached,
            lock_reason: self.lock_reason,
        }))
    }
}

fn registered_worktrees(root: &Path) -> Result<Vec<RegisteredWorktree>, GitError> {
    let output = git_output(root, &["worktree", "list", "--porcelain", "-z"])?;
    if !output.status.success() {
        return Err(command_error("list recovery worktrees", &output));
    }
    let mut records = Vec::new();
    let mut current = WorktreeRecordBuilder::default();
    for field in output.stdout.split(|byte| *byte == 0) {
        if field.is_empty() {
            if let Some(record) = current.finish()? {
                records.push(record);
            }
            current = WorktreeRecordBuilder::default();
            continue;
        }
        let field = String::from_utf8(field.to_vec()).map_err(|_| GitError::NonUtf8Output {
            operation: "list recovery worktrees",
        })?;
        if let Some(path) = field.strip_prefix("worktree ") {
            if current.path.is_some() {
                return Err(GitError::RecoveryStateConflict {
                    detail: "malformed Git worktree registry record".into(),
                });
            }
            current.path = Some(PathBuf::from(path));
        } else if let Some(head) = field.strip_prefix("HEAD ") {
            current.head = Some(CommitId::new(head.to_owned())?);
        } else if let Some(branch) = field.strip_prefix("branch ") {
            current.branch = Some(branch.to_owned());
        } else if field == "detached" {
            current.detached = true;
        } else if field == "locked" {
            current.lock_reason = Some(String::new());
        } else if let Some(reason) = field.strip_prefix("locked ") {
            current.lock_reason = Some(reason.to_owned());
        }
    }
    if let Some(record) = current.finish()? {
        records.push(record);
    }
    Ok(records)
}

fn ref_conflict(run: &RunWorkspace, actual: &CommitId, candidate: Option<&CommitId>) -> GitError {
    GitError::RecoveryStateConflict {
        detail: bounded(&format!(
            "retained ref expected parent `{}`{}; found `{actual}`",
            run.head_commit(),
            candidate.map_or_else(String::new, |commit| format!(" or candidate `{commit}`"))
        )),
    }
}

fn commit_conflict(expected: &CommitId, actual: &CommitId) -> GitError {
    GitError::RecoveryStateConflict {
        detail: bounded(&format!(
            "journal candidate commit expected `{expected}`, found `{actual}`"
        )),
    }
}

fn registration_mismatch(candidate: &CandidateWorkspace, detail: &str) -> GitError {
    GitError::WorktreeRegistrationMismatch {
        path: candidate.path().to_path_buf(),
        detail: bounded(detail),
    }
}
