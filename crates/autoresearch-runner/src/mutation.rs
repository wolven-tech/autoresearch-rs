//! Provider-neutral, bounded mutation requests and manual submission.

use autoresearch_config::{FrozenIdentity, ValidatedManifest};
use autoresearch_core::{
    CandidateCommit, CandidateDecision, CandidateWorkspace, MetricDirection, MutationBoundary,
    RepoPath, RepositoryInspector, RunWorkspace,
};
use autoresearch_git::{GitError, GitRepository, LockedGitRepository};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// One past candidate outcome supplied as context, never as authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriorDecision {
    /// One-based candidate number.
    pub index: u32,
    /// Hypothesis tested by that candidate.
    pub hypothesis: String,
    /// Frozen-policy decision.
    pub decision: CandidateDecision,
}

/// Human-readable objective and mandatory gates fixed before mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrozenRubric {
    /// Aggregate frozen-input digest.
    pub identity_sha256: String,
    /// Frozen program supplied by operator.
    pub program: String,
    /// Primary objective name.
    pub objective: String,
    /// Frozen improvement direction.
    pub objective_direction: MetricDirection,
    /// Mandatory gates in declared evaluator order.
    pub hard_gates: Vec<String>,
}

/// Small request passed to any agent adapter; no provider credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MutationRequest {
    /// Exact isolated candidate worktree.
    candidate_worktree: PathBuf,
    /// Frozen mutable roots, not broader than manifest scope.
    allowed_files: Vec<RepoPath>,
    /// Frozen objective, gates, program, and aggregate identity.
    frozen_rubric: FrozenRubric,
    /// Prior outcomes for learning, without changing selection policy.
    prior_decisions: Vec<PriorDecision>,
    /// Testable proposed change for this candidate.
    hypothesis: String,
    /// Run-owned cancellation handle, not a network token.
    cancellation_id: String,
    #[serde(skip)]
    boundary: MutationBoundary,
    #[serde(skip)]
    manifest_sha256: String,
}

/// Invalid request or unsafe manual submission.
#[derive(Debug, Error)]
pub enum MutationError {
    /// Required hypothesis is absent.
    #[error("candidate hypothesis is required")]
    MissingHypothesis,
    /// Program/rubric is absent.
    #[error("frozen program is required")]
    MissingProgram,
    /// Identity or cancellation handle has invalid shape.
    #[error("invalid frozen identity or cancellation identifier")]
    InvalidIdentity,
    /// Request does not belong to exact prepared worktree.
    #[error("mutation request does not match prepared candidate")]
    CandidateMismatch,
    /// Caller checkout changed during manual editing.
    #[error("caller checkout changed during manual mutation")]
    CallerChanged,
    /// Git containment, identity, or commit check failed.
    #[error(transparent)]
    Git(#[from] GitError),
}

impl MutationRequest {
    /// Returns exact candidate worktree path.
    #[must_use]
    pub fn candidate_worktree(&self) -> &Path {
        &self.candidate_worktree
    }

    /// Returns frozen mutable roots.
    #[must_use]
    pub fn allowed_files(&self) -> &[RepoPath] {
        &self.allowed_files
    }

    /// Returns human and machine rubric from frozen inputs.
    #[must_use]
    pub const fn frozen_rubric(&self) -> &FrozenRubric {
        &self.frozen_rubric
    }

    /// Returns prior policy outcomes.
    #[must_use]
    pub fn prior_decisions(&self) -> &[PriorDecision] {
        &self.prior_decisions
    }

    /// Returns candidate hypothesis.
    #[must_use]
    pub fn hypothesis(&self) -> &str {
        &self.hypothesis
    }

    /// Returns cancellation handle.
    #[must_use]
    pub fn cancellation_id(&self) -> &str {
        &self.cancellation_id
    }

    /// Constructs request only from frozen manifest and prepared candidate.
    ///
    /// # Errors
    ///
    /// Rejects missing hypothesis, program, or stable identifiers.
    pub fn new(
        candidate: &CandidateWorkspace,
        manifest: &ValidatedManifest,
        program: &str,
        frozen_identity: &FrozenIdentity,
        prior_decisions: Vec<PriorDecision>,
        hypothesis: &str,
        cancellation_id: &str,
    ) -> Result<Self, MutationError> {
        if hypothesis.trim().is_empty() {
            return Err(MutationError::MissingHypothesis);
        }
        if program.trim().is_empty() {
            return Err(MutationError::MissingProgram);
        }
        let comparison =
            FrozenIdentity::capture(manifest, program.as_bytes(), &BTreeMap::new(), None)
                .map_err(|_| MutationError::InvalidIdentity)?;
        if comparison.manifest != frozen_identity.manifest
            || comparison.program != frozen_identity.program
            || frozen_identity.aggregate_sha256.len() != 64
            || !frozen_identity
                .aggregate_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || cancellation_id.is_empty()
            || cancellation_id.len() > 128
            || !cancellation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(MutationError::InvalidIdentity);
        }
        let mut hard_gates = Vec::new();
        for evaluator in manifest.evaluators() {
            hard_gates.extend(evaluator.hard_gates().iter().cloned());
        }
        Ok(Self {
            candidate_worktree: candidate.path().to_path_buf(),
            allowed_files: manifest.scope().mutable_paths().to_vec(),
            frozen_rubric: FrozenRubric {
                identity_sha256: frozen_identity.aggregate_sha256.clone(),
                program: program.into(),
                objective: manifest.experiment().objective().name().into(),
                objective_direction: manifest.experiment().objective().direction(),
                hard_gates,
            },
            prior_decisions,
            hypothesis: hypothesis.trim().into(),
            cancellation_id: cancellation_id.into(),
            boundary: manifest.scope().clone(),
            manifest_sha256: frozen_identity.manifest.sha256.clone(),
        })
    }

    /// Returns exact frozen scope used at manual commit boundary.
    #[must_use]
    pub const fn boundary(&self) -> &MutationBoundary {
        &self.boundary
    }

    /// Checks manifest supplied at execution against request source identity.
    #[must_use]
    pub fn matches_manifest(&self, manifest: &ValidatedManifest) -> bool {
        FrozenIdentity::capture(
            manifest,
            self.frozen_rubric.program.as_bytes(),
            &BTreeMap::new(),
            None,
        )
        .is_ok_and(|identity| identity.manifest.sha256 == self.manifest_sha256)
    }
}

/// Commits user edits in prepared isolated worktree only. Does not evaluate,
/// select, or advance retained run ref. Phase 2 Git adapter owns all path,
/// symlink, nested-repository, and exact-parent validation.
///
/// # Errors
///
/// Rejects mismatched worktree/request, dirty caller checkout, or any frozen
/// scope breach before candidate can enter evaluation.
pub fn submit_manual_candidate(
    repository: &Path,
    git: &LockedGitRepository<'_>,
    run: &RunWorkspace,
    candidate: &CandidateWorkspace,
    request: &MutationRequest,
) -> Result<CandidateCommit, MutationError> {
    if request.candidate_worktree != candidate.path()
        || request.allowed_files != request.boundary.mutable_paths()
    {
        return Err(MutationError::CandidateMismatch);
    }
    let snapshot = GitRepository
        .inspect(repository, run.base_commit().as_str())
        .map_err(|error| match error {
            GitError::DirtyRepository { .. } => MutationError::CallerChanged,
            other => MutationError::Git(other),
        })?;
    if snapshot.root() != git.repository_root()
        || snapshot.base_commit() != run.base_commit()
        || snapshot.head_commit() != run.base_commit()
    {
        return Err(MutationError::CallerChanged);
    }
    git.commit_candidate(run, candidate, request.boundary())
        .map_err(Into::into)
}
