//! Containment checks and hook-free candidate commit creation.

use super::identity::ensure_candidate_repository_identity;
use super::{LockedGitRepository, ensure_candidate_clean, ensure_direct_child, resolve_commit};
use crate::GitError;
use crate::repository::{bounded, command_error, git_output, git_text};
use autoresearch_core::{
    CandidateCommit, CandidateCommitter, CandidateWorkspace, CommitId, Complexity,
    MutationBoundary, RepoPath, RunWorkspace,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

impl LockedGitRepository<'_> {
    /// Measures exact line delta from candidate commit for lexicographic
    /// tie-breaker. Dependency-manifest mutations fail closed until a
    /// dependency-aware delta adapter is available.
    ///
    /// # Errors
    ///
    /// Rejects mismatched commit, binary diff, overflow, and dependency files
    /// whose count cannot currently be represented truthfully.
    pub fn candidate_complexity(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        committed: &CandidateCommit,
        boundary: &MutationBoundary,
        runtime_ms: u64,
    ) -> Result<Complexity, GitError> {
        self.validate_candidate_commit(run, candidate, committed, boundary)?;
        for path in committed.changed_paths() {
            let name = path.as_str().rsplit('/').next().unwrap_or(path.as_str());
            if matches!(
                name,
                "Cargo.toml"
                    | "Cargo.lock"
                    | "package.json"
                    | "package-lock.json"
                    | "pnpm-lock.yaml"
                    | "yarn.lock"
                    | "bun.lock"
                    | "bun.lockb"
                    | "pyproject.toml"
                    | "uv.lock"
                    | "requirements.txt"
                    | "go.mod"
                    | "go.sum"
            ) {
                return Err(GitError::DependencyDeltaUnavailable {
                    path: path.as_str().into(),
                });
            }
        }
        let output = git_output(
            candidate.path(),
            &[
                "diff",
                "--numstat",
                "--no-renames",
                "-z",
                candidate.parent_commit().as_str(),
                committed.commit_id().as_str(),
                "--",
            ],
        )?;
        if !output.status.success() {
            return Err(command_error("measure candidate line delta", &output));
        }
        let mut lines = 0_u64;
        for record in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|r| !r.is_empty())
        {
            let record = std::str::from_utf8(record).map_err(|_| GitError::NonUtf8Output {
                operation: "measure candidate line delta",
            })?;
            let mut fields = record.splitn(3, '\t');
            let added = fields.next().and_then(|value| value.parse::<u64>().ok());
            let removed = fields.next().and_then(|value| value.parse::<u64>().ok());
            let path = fields.next();
            let (Some(added), Some(removed), Some(_)) = (added, removed, path) else {
                return Err(GitError::CandidateMutationRace {
                    detail: "candidate line delta is unavailable or binary".into(),
                });
            };
            lines = lines
                .checked_add(added)
                .and_then(|value| value.checked_add(removed))
                .ok_or_else(|| GitError::CandidateMutationRace {
                    detail: "candidate line delta overflowed".into(),
                })?;
        }
        Ok(Complexity {
            changed_lines: lines,
            dependency_delta: 0,
            runtime_ms,
        })
    }

    /// Rechecks exact candidate HEAD and committed path evidence before and
    /// after evaluators run. A changed commit or worktree cannot be scored.
    ///
    /// # Errors
    ///
    /// Rejects changed HEAD, dirty candidate, foreign Git identity, or diff
    /// paths inconsistent with original contained commit evidence.
    pub fn validate_candidate_commit(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        committed: &CandidateCommit,
        boundary: &MutationBoundary,
    ) -> Result<(), GitError> {
        self.ensure_candidate_current(run, candidate)?;
        ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
        ensure_candidate_clean(candidate.path())?;
        let head = resolve_commit(candidate.path(), "resolve candidate HEAD", "HEAD")?;
        if &head != committed.commit_id() {
            return Err(GitError::CandidateTopology {
                detail: "candidate HEAD differs from committed evidence".into(),
            });
        }
        ensure_direct_child(candidate.path(), &head, candidate.parent_commit())?;
        let paths = collect_commit_paths(candidate.path(), candidate.parent_commit(), &head)?;
        let actual = CandidateCommit::new(head, paths, boundary)?;
        if actual != *committed {
            return Err(GitError::CandidateMutationRace {
                detail: "candidate diff differs from committed evidence".into(),
            });
        }
        Ok(())
    }

    /// Validates all candidate mutations and creates one direct-child commit.
    ///
    /// # Errors
    ///
    /// Rejects stale or foreign candidate identity, empty or uncontained diffs,
    /// ignored files, symbolic links, nested repositories, and concurrent
    /// mutation. Candidate worktree remains available for later evaluation.
    pub fn commit_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        boundary: &MutationBoundary,
    ) -> Result<CandidateCommit, GitError> {
        self.ensure_candidate_current(run, candidate)?;
        ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
        let head = resolve_commit(candidate.path(), "resolve candidate HEAD", "HEAD")?;
        if &head != candidate.parent_commit() {
            return Err(GitError::CandidateTopology {
                detail: format!(
                    "candidate HEAD `{head}` must equal uncommitted parent `{}`",
                    candidate.parent_commit()
                ),
            });
        }

        reject_ignored_paths(candidate.path())?;
        let changed = collect_changed_paths(candidate.path())?;
        if changed.is_empty() {
            return Err(GitError::CandidateUnchanged);
        }
        validate_changed_paths(
            candidate.path(),
            candidate.parent_commit(),
            &changed,
            boundary,
        )?;

        git_text(
            candidate.path(),
            "stage candidate worktree",
            &["add", "--all", "--"],
        )?;

        reject_ignored_paths(candidate.path())?;
        let staged = collect_staged_paths(candidate.path())?;
        let pending = collect_unstaged_or_untracked_paths(candidate.path())?;
        if staged != changed || !pending.is_empty() {
            return Err(GitError::CandidateMutationRace {
                detail: bounded(&format!(
                    "before [{}], staged [{}], pending [{}]",
                    display_paths(&changed),
                    display_paths(&staged),
                    display_paths(&pending)
                )),
            });
        }
        validate_changed_paths(
            candidate.path(),
            candidate.parent_commit(),
            &staged,
            boundary,
        )?;

        let tree = git_text(candidate.path(), "write candidate tree", &["write-tree"])?;
        let message = format!(
            "autoresearch({}): candidate {:06}",
            run.run_id(),
            candidate.index()
        );
        let commit = CommitId::new(git_text(
            candidate.path(),
            "create candidate commit",
            &[
                "commit-tree",
                &tree,
                "-p",
                candidate.parent_commit().as_str(),
                "-m",
                &message,
            ],
        )?)?;
        ensure_direct_child(candidate.path(), &commit, candidate.parent_commit())?;
        let committed = collect_commit_paths(candidate.path(), candidate.parent_commit(), &commit)?;
        if committed != staged {
            return Err(GitError::CandidateMutationRace {
                detail: bounded(&format!(
                    "staged [{}], committed [{}]",
                    display_paths(&staged),
                    display_paths(&committed)
                )),
            });
        }
        let evidence = CandidateCommit::new(commit.clone(), committed, boundary)?;

        let output = git_output(
            candidate.path(),
            &[
                "update-ref",
                "--no-deref",
                "HEAD",
                commit.as_str(),
                candidate.parent_commit().as_str(),
            ],
        )?;
        if !output.status.success() {
            return Err(command_error("advance candidate HEAD", &output));
        }
        ensure_candidate_clean(candidate.path())?;
        ensure_candidate_repository_identity(self.snapshot.root(), candidate)?;
        Ok(evidence)
    }
}

impl CandidateCommitter for LockedGitRepository<'_> {
    type Error = GitError;

    fn commit_candidate(
        &self,
        run: &RunWorkspace,
        candidate: &CandidateWorkspace,
        boundary: &MutationBoundary,
    ) -> Result<CandidateCommit, Self::Error> {
        Self::commit_candidate(self, run, candidate, boundary)
    }
}

fn collect_changed_paths(root: &Path) -> Result<Vec<RepoPath>, GitError> {
    let mut paths = BTreeSet::new();
    extend_paths(
        &mut paths,
        root,
        "list unstaged candidate paths",
        &["diff", "--name-only", "--no-renames", "-z", "--"],
    )?;
    extend_paths(
        &mut paths,
        root,
        "list staged candidate paths",
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
            "--",
        ],
    )?;
    extend_paths(
        &mut paths,
        root,
        "list untracked candidate paths",
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?;
    Ok(paths.into_iter().collect())
}

fn collect_staged_paths(root: &Path) -> Result<Vec<RepoPath>, GitError> {
    git_nul_paths(
        root,
        "list staged candidate paths",
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
            "--",
        ],
    )
}

fn collect_unstaged_or_untracked_paths(root: &Path) -> Result<Vec<RepoPath>, GitError> {
    let mut paths = BTreeSet::new();
    extend_paths(
        &mut paths,
        root,
        "list unstaged candidate paths",
        &["diff", "--name-only", "--no-renames", "-z", "--"],
    )?;
    extend_paths(
        &mut paths,
        root,
        "list untracked candidate paths",
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?;
    Ok(paths.into_iter().collect())
}

fn collect_commit_paths(
    root: &Path,
    parent: &CommitId,
    commit: &CommitId,
) -> Result<Vec<RepoPath>, GitError> {
    git_nul_paths(
        root,
        "list committed candidate paths",
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            parent.as_str(),
            commit.as_str(),
            "--",
        ],
    )
}

fn extend_paths(
    paths: &mut BTreeSet<RepoPath>,
    root: &Path,
    operation: &'static str,
    args: &[&str],
) -> Result<(), GitError> {
    paths.extend(git_nul_paths(root, operation, args)?);
    Ok(())
}

fn git_nul_paths(
    root: &Path,
    operation: &'static str,
    args: &[&str],
) -> Result<Vec<RepoPath>, GitError> {
    let output = git_output(root, args)?;
    if !output.status.success() {
        return Err(command_error(operation, &output));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = path.strip_suffix(b"/").unwrap_or(path);
            let path = String::from_utf8(path.to_vec())
                .map_err(|_| GitError::NonUtf8Output { operation })?;
            RepoPath::new(path).map_err(Into::into)
        })
        .collect()
}

fn reject_ignored_paths(root: &Path) -> Result<(), GitError> {
    let ignored = git_nul_paths(
        root,
        "list ignored candidate paths",
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
            "--",
        ],
    )?;
    if ignored.is_empty() {
        Ok(())
    } else {
        Err(GitError::IgnoredCandidatePaths {
            paths: bounded(&display_paths(&ignored)),
        })
    }
}

fn validate_changed_paths(
    root: &Path,
    parent: &CommitId,
    paths: &[RepoPath],
    boundary: &MutationBoundary,
) -> Result<(), GitError> {
    for path in paths {
        boundary.validate(path)?;
        ensure_path_shape(root, path)?;
        reject_forbidden_tree_modes(root, parent, path)?;
        reject_forbidden_index_modes(root, path)?;
    }
    Ok(())
}

fn ensure_path_shape(root: &Path, path: &RepoPath) -> Result<(), GitError> {
    let segments = path.as_str().split('/').collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    let mut relative = String::new();
    for (index, segment) in segments.iter().enumerate() {
        current.push(segment);
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(segment);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(GitError::CandidateSymlink {
                    path: relative.clone(),
                });
            }
            Ok(metadata) if metadata.is_dir() => {
                if fs::symlink_metadata(current.join(".git")).is_ok() {
                    return Err(GitError::NestedRepository {
                        path: relative.clone(),
                    });
                }
            }
            Ok(_) if index + 1 < segments.len() => {
                return Err(GitError::UnsafeCandidateEntry {
                    path: relative.clone(),
                    reason: "non-directory entry appears before path end",
                });
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(GitError::UnsafeCandidateEntry {
                    path: relative.clone(),
                    reason: "changed entry must be a regular file, directory, or deletion",
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => {
                return Err(GitError::Io {
                    operation: "inspect candidate path",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn reject_forbidden_tree_modes(
    root: &Path,
    parent: &CommitId,
    path: &RepoPath,
) -> Result<(), GitError> {
    reject_forbidden_modes(
        root,
        path,
        "inspect candidate parent mode",
        &["ls-tree", "-z", parent.as_str(), "--", path.as_str()],
    )
}

fn reject_forbidden_index_modes(root: &Path, path: &RepoPath) -> Result<(), GitError> {
    reject_forbidden_modes(
        root,
        path,
        "inspect candidate index mode",
        &["ls-files", "--stage", "-z", "--", path.as_str()],
    )
}

fn reject_forbidden_modes(
    root: &Path,
    path: &RepoPath,
    operation: &'static str,
    args: &[&str],
) -> Result<(), GitError> {
    let output = git_output(root, args)?;
    if !output.status.success() {
        return Err(command_error(operation, &output));
    }
    let listing =
        String::from_utf8(output.stdout).map_err(|_| GitError::NonUtf8Output { operation })?;
    for entry in listing.split('\0').filter(|entry| !entry.is_empty()) {
        let mode = entry.split_once(' ').map_or(entry, |(mode, _)| mode);
        if matches!(mode, "120000" | "160000") {
            return Err(GitError::ForbiddenCandidateMode {
                path: path.as_str().to_owned(),
                mode: mode.to_owned(),
            });
        }
    }
    Ok(())
}

fn display_paths(paths: &[RepoPath]) -> String {
    paths
        .iter()
        .map(RepoPath::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
