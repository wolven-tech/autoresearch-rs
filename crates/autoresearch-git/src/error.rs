//! Typed failures shared by Git infrastructure adapters.

use autoresearch_core::{
    CandidateCommitError, ContainmentViolation, RepoPathError, RepositoryValueError,
    WorkspaceValueError,
};
use std::path::PathBuf;
use thiserror::Error;

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
    /// Run ID cannot be represented safely in lock metadata.
    #[error("run ID must be 1-128 ASCII letters, digits, dashes, or underscores")]
    InvalidRunId,
    /// Ignored run-state path is structurally unsafe.
    #[error("unsafe run-state path {}: {reason}", path.display())]
    UnsafeRunState {
        /// Unsafe path.
        path: PathBuf,
        /// Stable refusal reason.
        reason: &'static str,
    },
    /// Repository has another active experiment owner.
    #[error("repository lock is held at {}: {detail}", path.display())]
    LockHeld {
        /// Lock evidence path.
        path: PathBuf,
        /// Bounded owner metadata, when readable.
        detail: String,
    },
    /// Repository identity moved after caller captured its snapshot.
    #[error("repository changed between validation and lock acquisition")]
    StaleSnapshot,
    /// Lock belongs to another repository snapshot.
    #[error("repository lock does not belong to requested repository")]
    LockRepositoryMismatch,
    /// Retained run branch does not descend from frozen base.
    #[error("run branch `{branch_ref}` at `{head_commit}` does not descend from `{base_commit}`")]
    RunBranchDiverged {
        /// Full retained branch ref.
        branch_ref: String,
        /// Frozen base commit.
        base_commit: String,
        /// Conflicting branch head.
        head_commit: String,
    },
    /// Retained branch is checked out and cannot be managed only through refs.
    #[error("run branch `{branch_ref}` is checked out at {}", worktree.display())]
    RunBranchCheckedOut {
        /// Full retained branch ref.
        branch_ref: String,
        /// Worktree holding branch.
        worktree: PathBuf,
    },
    /// Supplied run value belongs to another adapter or frozen base.
    #[error("run workspace does not belong to locked repository")]
    ForeignRunWorkspace,
    /// Retained ref moved or disappeared after run value was opened.
    #[error("run branch changed: expected `{expected}`, found `{actual}`")]
    StaleRunBranch {
        /// Commit expected by caller's run value.
        expected: String,
        /// Current commit or explicit missing marker.
        actual: String,
    },
    /// Candidate path already exists and cannot be overwritten.
    #[error("candidate worktree path already exists: {}", .0.display())]
    WorktreePathExists(PathBuf),
    /// Candidate worktree parent or target could escape through unsafe path shape.
    #[error("unsafe worktree path {}: {reason}", path.display())]
    UnsafeWorktreePath {
        /// Unsafe path.
        path: PathBuf,
        /// Stable refusal reason.
        reason: &'static str,
    },
    /// Candidate value was not produced for supplied run and adapter.
    #[error("candidate workspace does not belong to supplied run")]
    ForeignCandidateWorkspace,
    /// Candidate is attached to a branch instead of detached commit.
    #[error("candidate worktree is attached to branch `{branch}`")]
    CandidateNotDetached {
        /// Checked-out branch ref.
        branch: String,
    },
    /// Candidate contains staged, tracked, or untracked changes.
    #[error("candidate worktree must be clean before retention: {status}")]
    DirtyCandidate {
        /// Bounded porcelain status.
        status: String,
    },
    /// Candidate commit is not one direct non-merge child of retained head.
    #[error("candidate commit topology invalid: {detail}")]
    CandidateTopology {
        /// Bounded topology diagnostic.
        detail: String,
    },
    /// Candidate contains no Git-visible mutation.
    #[error("candidate has no changed paths to commit")]
    CandidateUnchanged,
    /// Candidate changed state ignored by Git and therefore absent from commit.
    #[error("candidate contains ignored paths that cannot enter commit: {paths}")]
    IgnoredCandidatePaths {
        /// Bounded ignored-path list.
        paths: String,
    },
    /// Candidate path or ancestor is a symbolic link.
    #[error("candidate changed symbolic-link path `{path}`")]
    CandidateSymlink {
        /// Repository-relative rejected path.
        path: String,
    },
    /// Candidate contains a nested Git repository.
    #[error("candidate contains nested Git repository at `{path}`")]
    NestedRepository {
        /// Repository-relative repository root.
        path: String,
    },
    /// Candidate index entry cannot be represented as a regular file.
    #[error("candidate path `{path}` has forbidden Git mode `{mode}`")]
    ForbiddenCandidateMode {
        /// Repository-relative path.
        path: String,
        /// Git tree or index mode.
        mode: String,
    },
    /// Candidate filesystem entry is not a regular file or directory.
    #[error("unsafe candidate entry `{path}`: {reason}")]
    UnsafeCandidateEntry {
        /// Repository-relative path.
        path: String,
        /// Stable refusal reason.
        reason: &'static str,
    },
    /// Candidate Git metadata no longer identifies expected linked worktree.
    #[error("candidate Git identity mismatch: {detail}")]
    CandidateRepositoryMismatch {
        /// Bounded mismatch explanation.
        detail: String,
    },
    /// Candidate changed concurrently while commit was being assembled.
    #[error("candidate changed while commit was being assembled: {detail}")]
    CandidateMutationRace {
        /// Bounded changed-path evidence.
        detail: String,
    },
    /// Lock filesystem operation failed.
    #[error("failed to {operation} at {}: {source}", path.display())]
    LockIo {
        /// Stable operation description.
        operation: &'static str,
        /// Affected lock path.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// System clock cannot provide lock acquisition time.
    #[error("system clock is before Unix epoch")]
    ClockBeforeEpoch,
    /// Millisecond timestamp cannot fit stable lock schema.
    #[error("system timestamp exceeds lock schema range")]
    ClockOverflow,
    /// Lock owner metadata could not be encoded.
    #[error("failed to encode lock owner: {0}")]
    LockSerialization(#[from] serde_json::Error),
    /// Domain repository identity returned by Git was invalid.
    #[error(transparent)]
    InvalidIdentity(#[from] RepositoryValueError),
    /// Domain workspace identity derived by adapter was invalid.
    #[error(transparent)]
    InvalidWorkspace(#[from] WorkspaceValueError),
    /// Git emitted an invalid repository-relative candidate path.
    #[error(transparent)]
    InvalidCandidatePath(#[from] RepoPathError),
    /// Candidate path violates frozen mutable/protected roots.
    #[error(transparent)]
    Containment(#[from] ContainmentViolation),
    /// Adapter tried to return malformed candidate commit evidence.
    #[error(transparent)]
    InvalidCandidateCommit(#[from] CandidateCommitError),
}
