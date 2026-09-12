//! Contract coverage for workspace lifecycle domain values.

use autoresearch_core::{CandidateWorkspace, CommitId, RunId, RunWorkspace, WorkspaceValueError};
use std::path::{Path, PathBuf};

const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
const NEXT: &str = "fedcba9876543210fedcba9876543210fedcba98";

#[test]
fn run_id_is_path_and_ref_safe() {
    let run_id = RunId::new("run-20260912_001").expect("valid run ID");
    assert_eq!(run_id.as_str(), "run-20260912_001");
    assert_eq!(run_id.to_string(), "run-20260912_001");

    for invalid in ["", ".", "..", "run/escape", "run.with.dot", "run space"] {
        assert_eq!(RunId::new(invalid), Err(WorkspaceValueError::InvalidRunId));
    }
}

#[test]
fn run_workspace_derives_canonical_ref_and_advances_immutably() {
    let run = fixture_run();
    assert_eq!(run.branch_ref(), "refs/heads/autoresearch/run-20260912_001");
    assert_eq!(run.base_commit().as_str(), BASE);
    assert_eq!(run.head_commit().as_str(), BASE);

    let advanced = run.advanced_to(commit(NEXT));
    assert_eq!(advanced.run_id(), run.run_id());
    assert_eq!(advanced.base_commit(), run.base_commit());
    assert_eq!(advanced.head_commit().as_str(), NEXT);
    assert_eq!(run.head_commit().as_str(), BASE);
}

#[test]
fn candidate_identity_and_path_are_deterministic() {
    let run = fixture_run();
    let candidate =
        CandidateWorkspace::new(&run, 7, Path::new("/tmp/worktrees")).expect("candidate workspace");

    assert_eq!(candidate.run_id(), run.run_id());
    assert_eq!(candidate.index(), 7);
    assert_eq!(candidate.worktree_id(), "candidate-000007");
    assert_eq!(
        candidate.path(),
        Path::new("/tmp/worktrees/run-20260912_001/candidate-000007")
    );
    assert_eq!(candidate.parent_commit(), run.head_commit());
}

#[test]
fn candidate_rejects_zero_or_unsafe_root() {
    let run = fixture_run();
    assert_eq!(
        CandidateWorkspace::new(&run, 0, Path::new("/tmp/worktrees")),
        Err(WorkspaceValueError::ZeroCandidateIndex)
    );
    assert_eq!(
        CandidateWorkspace::new(&run, 1, Path::new("worktrees")),
        Err(WorkspaceValueError::RelativeWorktreeRoot)
    );
    assert_eq!(
        CandidateWorkspace::new(&run, 1, Path::new("/")),
        Err(WorkspaceValueError::FilesystemRoot)
    );
    assert_eq!(
        CandidateWorkspace::new(&run, 1, &PathBuf::from("/tmp/../worktrees")),
        Err(WorkspaceValueError::UnnormalizedWorktreeRoot)
    );
}

#[test]
fn run_id_deserialization_revalidates_input() {
    let run_id: RunId = serde_json::from_str("\"run-1\"").expect("valid JSON run ID");
    assert_eq!(run_id.as_str(), "run-1");
    assert!(serde_json::from_str::<RunId>("\"../escape\"").is_err());
}

fn fixture_run() -> RunWorkspace {
    RunWorkspace::new(
        RunId::new("run-20260912_001").expect("valid run ID"),
        commit(BASE),
        commit(BASE),
    )
}

fn commit(value: &str) -> CommitId {
    CommitId::new(value).expect("valid commit")
}
