//! Read-only exact-commit diff evidence for complexity tie-breakers.

use crate::EvaluationContext;
use autoresearch_core::{
    CommitId, Complexity, ContainmentViolation, MutationBoundary, RepoPath, RepoPathError,
};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use thiserror::Error;

const MAX_GIT_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Dependency format declared by caller; no format guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyManifest {
    /// Root-package Cargo.toml with direct dependency tables only.
    CargoToml(RepoPath),
    /// Explicit unsupported format, kept as unavailable evidence.
    Unsupported(String),
}

/// Evidence for dependency-delta tie-breaker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyEvidence {
    /// Newly introduced direct dependency names in lexical order.
    Known {
        /// New dependency names, unique across supported tables.
        added: Vec<String>,
    },
    /// No supported, trustworthy dependency comparison exists.
    Unavailable {
        /// Stable, non-secret explanation.
        reason: String,
    },
}

/// Exact diff plus optional dependency accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEvidence {
    changed_paths: Vec<RepoPath>,
    binary_paths: Vec<RepoPath>,
    changed_lines: u64,
    dependency_evidence: DependencyEvidence,
}

impl DiffEvidence {
    /// Changed old and new paths, sorted and unique; rename includes both.
    #[must_use]
    pub fn changed_paths(&self) -> &[RepoPath] {
        &self.changed_paths
    }

    /// Binary paths with no invented line count.
    #[must_use]
    pub fn binary_paths(&self) -> &[RepoPath] {
        &self.binary_paths
    }

    /// Text additions plus deletions from exact committed tree diff.
    #[must_use]
    pub const fn changed_lines(&self) -> u64 {
        self.changed_lines
    }

    /// Supported or explicitly unavailable dependency comparison.
    #[must_use]
    pub const fn dependency_evidence(&self) -> &DependencyEvidence {
        &self.dependency_evidence
    }

    /// Builds core tie-breaker input only when all fields are known.
    ///
    /// # Errors
    ///
    /// Unsupported or unreadable dependency manifests cannot become zero.
    pub fn complexity(&self, runtime_ms: u64) -> Result<Complexity, DiffError> {
        let DependencyEvidence::Known { added } = &self.dependency_evidence else {
            return Err(DiffError::DependencyUnavailable);
        };
        let dependency_delta = u32::try_from(added.len()).map_err(|_| DiffError::CountOverflow)?;
        Ok(Complexity {
            changed_lines: self.changed_lines,
            dependency_delta,
            runtime_ms,
        })
    }
}

/// Exact-diff failure. No variant returns a fabricated score or complexity.
#[derive(Debug, Error)]
pub enum DiffError {
    /// Git command, repository identity, or output failure.
    #[error("Git diff evidence unavailable: {0}")]
    Git(String),
    /// Evaluated commit does not directly descend from declared parent.
    #[error("candidate commit topology mismatch: {0}")]
    Topology(String),
    /// Evaluator-visible checkout differs from committed tree.
    #[error("candidate worktree is dirty")]
    DirtyWorktree,
    /// Declared paths do not equal committed diff paths.
    #[error("declared changed paths differ from exact committed diff")]
    ChangedPathsMismatch,
    /// No committed change exists.
    #[error("candidate commit has no changed paths")]
    EmptyDiff,
    /// Changed path is not safe and portable.
    #[error(transparent)]
    InvalidPath(#[from] RepoPathError),
    /// Changed path violates frozen mutation boundary.
    #[error(transparent)]
    Containment(#[from] ContainmentViolation),
    /// Git numstat record cannot be interpreted safely.
    #[error("malformed Git numstat output")]
    MalformedNumstat,
    /// Numeric count exceeds core type.
    #[error("diff count exceeds supported range")]
    CountOverflow,
    /// Dependency comparison is not supported or available.
    #[error("dependency comparison unavailable")]
    DependencyUnavailable,
}

/// Derives diff and dependency evidence from exact committed trees.
///
/// `parent_commit` may be current-best commit rather than run baseline, but
/// must be sole parent of `context.evaluated_commit()`. Candidate HEAD must
/// equal evaluated commit, candidate worktree must be clean, and declared
/// changed paths must match derived paths. No checkout or repository file is
/// modified. `git` is invoked with literal arguments and no shell.
///
/// # Errors
///
/// Rejects wrong topology, dirty checkout, path mismatch, malformed Git
/// output, or protected/out-of-scope changed paths.
pub fn evaluate_commit_diff(
    context: &EvaluationContext,
    parent_commit: &CommitId,
    boundary: &MutationBoundary,
    dependency_manifest: DependencyManifest,
) -> Result<DiffEvidence, DiffError> {
    let root = context.candidate_worktree();
    let top = git_text(root, &["rev-parse", "--show-toplevel"])?;
    if Path::new(&top) != root {
        return Err(DiffError::Git(
            "candidate path is not Git worktree root".into(),
        ));
    }
    let head = git_text(root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if head != context.evaluated_commit().as_str() {
        return Err(DiffError::Topology(
            "candidate HEAD differs from evaluated commit".into(),
        ));
    }
    let ancestry = git_text(root, &["rev-list", "--parents", "-n", "1", head.as_str()])?;
    let mut commits = ancestry.split_ascii_whitespace();
    if commits.next() != Some(head.as_str())
        || commits.next() != Some(parent_commit.as_str())
        || commits.next().is_some()
    {
        return Err(DiffError::Topology(
            "candidate is not direct child of declared parent".into(),
        ));
    }
    if !git_bytes(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )?
    .is_empty()
        || !git_bytes(
            root,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
                "--",
            ],
        )?
        .is_empty()
    {
        return Err(DiffError::DirtyWorktree);
    }

    let bytes = git_bytes(
        root,
        &[
            "diff-tree",
            "--no-commit-id",
            "--numstat",
            "-r",
            "-z",
            "--find-renames",
            "--no-ext-diff",
            "--no-textconv",
            parent_commit.as_str(),
            context.evaluated_commit().as_str(),
            "--",
        ],
    )?;
    let (changed_paths, binary_paths, changed_lines) = parse_numstat(&bytes)?;
    if changed_paths.is_empty() {
        return Err(DiffError::EmptyDiff);
    }
    for path in &changed_paths {
        boundary.validate(path)?;
    }
    let mut declared = context.changed_paths().to_vec();
    declared.sort();
    if declared != changed_paths {
        return Err(DiffError::ChangedPathsMismatch);
    }
    let dependency_evidence = compare_dependencies(
        root,
        parent_commit,
        context.evaluated_commit(),
        dependency_manifest,
    );
    Ok(DiffEvidence {
        changed_paths,
        binary_paths,
        changed_lines,
        dependency_evidence,
    })
}

fn parse_numstat(bytes: &[u8]) -> Result<(Vec<RepoPath>, Vec<RepoPath>, u64), DiffError> {
    let mut records = bytes.split(|byte| *byte == 0).peekable();
    let mut changed_paths = BTreeSet::new();
    let mut binary_paths = BTreeSet::new();
    let mut changed_lines = 0_u64;
    while let Some(record) = records.next() {
        if record.is_empty() && records.peek().is_none() {
            break;
        }
        let mut columns = record.splitn(3, |byte| *byte == b'\t');
        let additions = columns.next().ok_or(DiffError::MalformedNumstat)?;
        let deletions = columns.next().ok_or(DiffError::MalformedNumstat)?;
        let path_column = columns.next().ok_or(DiffError::MalformedNumstat)?;
        let path = if path_column.is_empty() {
            let old = records.next().ok_or(DiffError::MalformedNumstat)?;
            changed_paths.insert(parse_path(old)?);
            records.next().ok_or(DiffError::MalformedNumstat)?
        } else {
            path_column
        };
        let path = parse_path(path)?;
        changed_paths.insert(path.clone());
        if additions == b"-" && deletions == b"-" {
            binary_paths.insert(path);
        } else {
            let additions = parse_count(additions)?;
            let deletions = parse_count(deletions)?;
            changed_lines = changed_lines
                .checked_add(additions)
                .and_then(|sum| sum.checked_add(deletions))
                .ok_or(DiffError::CountOverflow)?;
        }
    }
    Ok((
        changed_paths.into_iter().collect(),
        binary_paths.into_iter().collect(),
        changed_lines,
    ))
}

fn parse_count(bytes: &[u8]) -> Result<u64, DiffError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DiffError::MalformedNumstat)?;
    text.parse().map_err(|_| DiffError::MalformedNumstat)
}

fn parse_path(bytes: &[u8]) -> Result<RepoPath, DiffError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DiffError::MalformedNumstat)?;
    RepoPath::new(text.to_owned()).map_err(Into::into)
}

fn compare_dependencies(
    root: &Path,
    parent: &CommitId,
    candidate: &CommitId,
    declaration: DependencyManifest,
) -> DependencyEvidence {
    let DependencyManifest::CargoToml(path) = declaration else {
        return DependencyEvidence::Unavailable {
            reason: "declared dependency format is unsupported".into(),
        };
    };
    let read_names = |commit: &CommitId| {
        let reference = format!("{}:{}", commit.as_str(), path.as_str());
        let bytes = git_bytes(root, &["show", &reference]).ok()?;
        let source = String::from_utf8(bytes).ok()?;
        let document = toml::from_str::<toml::Value>(&source).ok()?;
        cargo_dependency_names(&document)
    };
    let (Some(before), Some(after)) = (read_names(parent), read_names(candidate)) else {
        return DependencyEvidence::Unavailable {
            reason: "declared Cargo manifest missing, invalid, or unsupported".into(),
        };
    };
    DependencyEvidence::Known {
        added: after.difference(&before).cloned().collect(),
    }
}

fn cargo_dependency_names(document: &toml::Value) -> Option<BTreeSet<String>> {
    let table = document.as_table()?;
    if !table.contains_key("package")
        || table.contains_key("workspace")
        || table.contains_key("target")
    {
        return None;
    }
    let mut names = BTreeSet::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(value) = table.get(section) {
            for (name, specification) in value.as_table()? {
                if specification
                    .as_table()
                    .is_some_and(|spec| spec.get("workspace").is_some())
                {
                    return None;
                }
                if !specification.is_str() && !specification.is_table() {
                    return None;
                }
                names.insert(name.clone());
            }
        }
    }
    Some(names)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, DiffError> {
    let bytes = git_bytes(root, args)?;
    String::from_utf8(bytes)
        .map(|value| value.trim().to_owned())
        .map_err(|_| DiffError::Git("Git returned non-UTF-8 identity".into()))
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, DiffError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|_| DiffError::Git("could not start Git".into()))?;
    if !output.status.success() {
        return Err(DiffError::Git(
            "Git could not read declared committed state".into(),
        ));
    }
    if output.stdout.len() > MAX_GIT_OUTPUT_BYTES || output.stderr.len() > MAX_GIT_OUTPUT_BYTES {
        return Err(DiffError::Git("Git output exceeds evidence limit".into()));
    }
    Ok(output.stdout)
}
