//! Contract coverage for pure mutation containment values.

use autoresearch_core::{
    CandidateCommit, CandidateCommitError, CommitId, ContainmentViolation, MutationBoundary,
    MutationBoundaryError, RepoPath,
};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn repository_paths_reject_ambiguous_or_escaping_text() {
    assert_eq!(
        RepoPath::new("web/src/page.rs")
            .expect("valid path")
            .as_str(),
        "web/src/page.rs"
    );
    for invalid in [
        "",
        "/tmp/outside",
        "../outside",
        "web/./src",
        "web//src",
        "web\\src",
        "web\nsrc",
    ] {
        assert!(RepoPath::new(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn boundary_normalizes_roots_and_uses_protected_carve_outs_first() {
    let boundary = boundary();
    assert_eq!(
        boundary
            .mutable_paths()
            .iter()
            .map(RepoPath::as_str)
            .collect::<Vec<_>>(),
        ["web", "web/src"]
    );
    assert!(boundary.validate(&path("web/src/page.rs")).is_ok());
    assert!(matches!(
        boundary.validate(&path("web/internal/key.rs")),
        Err(ContainmentViolation::ProtectedPath { .. })
    ));
    assert!(matches!(
        boundary.validate(&path("website/page.rs")),
        Err(ContainmentViolation::OutsideMutable { .. })
    ));
}

#[test]
fn boundary_rejects_empty_or_already_protected_mutable_roots() {
    assert_eq!(
        MutationBoundary::new(Vec::new(), vec![path("program.md")]),
        Err(MutationBoundaryError::EmptyMutablePaths)
    );
    assert!(matches!(
        MutationBoundary::new(vec![path("web/internal")], vec![path("web")]),
        Err(MutationBoundaryError::MutableProtected { .. })
    ));
}

#[test]
fn candidate_commit_canonicalizes_and_revalidates_changed_paths() {
    let boundary = boundary();
    let evidence = CandidateCommit::new(
        CommitId::new(COMMIT).expect("commit"),
        vec![path("web/src/z.rs"), path("web/a.rs"), path("web/a.rs")],
        &boundary,
    )
    .expect("contained evidence");
    assert_eq!(evidence.commit_id().as_str(), COMMIT);
    assert_eq!(
        evidence
            .changed_paths()
            .iter()
            .map(RepoPath::as_str)
            .collect::<Vec<_>>(),
        ["web/a.rs", "web/src/z.rs"]
    );
    assert_eq!(
        CandidateCommit::new(
            CommitId::new(COMMIT).expect("commit"),
            Vec::new(),
            &boundary
        ),
        Err(CandidateCommitError::NoChangedPaths)
    );
    assert!(matches!(
        CandidateCommit::new(
            CommitId::new(COMMIT).expect("commit"),
            vec![path("program.md")],
            &boundary
        ),
        Err(CandidateCommitError::Containment(
            ContainmentViolation::OutsideMutable { .. }
        ))
    ));
}

fn boundary() -> MutationBoundary {
    MutationBoundary::new(
        vec![path("web/src"), path("web"), path("web/src")],
        vec![path("web/internal")],
    )
    .expect("valid boundary")
}

fn path(value: &str) -> RepoPath {
    RepoPath::new(value).expect("valid repository path")
}
