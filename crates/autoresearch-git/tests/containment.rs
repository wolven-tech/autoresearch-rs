//! Adversarial integration coverage for contained candidate commits.

use autoresearch_core::{
    CandidateCommit, CandidateCommitter, CandidateWorkspace, ContainmentViolation,
    MutationBoundary, RepoPath, RepositoryInspector, RunWorkspace,
};
use autoresearch_git::{GitError, GitRepository, LockedGitRepository, RunLockGuard};
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRepository(PathBuf);

impl TestRepository {
    fn new() -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autoresearch-git-containment-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create fixture repository");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::write(root.join(".gitignore"), ".autoresearch/\nignored.tmp\n")
            .expect("write gitignore");
        fs::create_dir_all(root.join("mutable/protected")).expect("create mutable fixture");
        fs::write(root.join("mutable/edit.txt"), "baseline\n").expect("write editable file");
        fs::write(root.join("mutable/delete.txt"), "delete me\n").expect("write deleted file");
        fs::write(root.join("mutable/protected/control.txt"), "frozen\n")
            .expect("write protected file");
        fs::write(root.join("program.md"), "frozen program\n").expect("write program");
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        Self(root)
    }

    fn snapshot(&self) -> autoresearch_core::RepositorySnapshot {
        GitRepository
            .inspect(&self.0, "HEAD")
            .expect("inspect fixture")
    }

    fn caller_state(&self) -> (String, String, String, String) {
        (
            git_text(&self.0, &["branch", "--show-current"]),
            git_text(&self.0, &["rev-parse", "HEAD"]),
            git_text(&self.0, &["diff", "--cached", "--binary"]),
            git_text(
                &self.0,
                &["status", "--porcelain=v1", "--untracked-files=all"],
            ),
        )
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove fixture repository");
    }
}

#[test]
fn allowed_create_edit_delete_become_one_hook_free_candidate_commit() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let caller_before = repository.caller_state();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");
    fs::write(candidate.path().join("mutable/edit.txt"), "improved\n").expect("edit candidate");
    fs::remove_file(candidate.path().join("mutable/delete.txt")).expect("delete candidate file");
    fs::write(candidate.path().join("mutable/new.txt"), "new\n").expect("create candidate file");

    let evidence = commit_via_port(&adapter, &run, &candidate, &boundary());

    assert_ne!(evidence.commit_id(), candidate.parent_commit());
    assert_eq!(
        evidence
            .changed_paths()
            .iter()
            .map(RepoPath::as_str)
            .collect::<Vec<_>>(),
        ["mutable/delete.txt", "mutable/edit.txt", "mutable/new.txt"]
    );
    assert_eq!(
        git_text(candidate.path(), &["rev-parse", "HEAD"]),
        evidence.commit_id().as_str()
    );
    let candidate_range = format!("{}..HEAD", candidate.parent_commit());
    assert_eq!(
        git_text(candidate.path(), &["rev-list", "--count", &candidate_range]),
        "1"
    );
    assert_eq!(
        git_text(candidate.path(), &["log", "-1", "--format=%s"]),
        "autoresearch(run-1): candidate 000001"
    );
    assert!(
        git_text(candidate.path(), &["status", "--porcelain=v1"]).is_empty(),
        "candidate must be clean after commit"
    );
    assert_eq!(
        git_text(&repository.0, &["rev-parse", run.branch_ref()]),
        run.head_commit().as_str(),
        "commit must not retain candidate implicitly"
    );
    assert_eq!(repository.caller_state(), caller_before);
}

#[test]
fn protected_and_outside_changes_fail_before_head_moves() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let caller_before = repository.caller_state();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");

    let protected = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare protected candidate");
    fs::write(
        protected.path().join("mutable/protected/control.txt"),
        "tampered\n",
    )
    .expect("tamper protected path");
    assert!(matches!(
        adapter.commit_candidate(&run, &protected, &boundary()),
        Err(GitError::Containment(
            ContainmentViolation::ProtectedPath { .. }
        ))
    ));
    assert_head_unchanged(&protected);

    let outside = adapter
        .prepare_candidate(&run, 2)
        .expect("prepare outside candidate");
    fs::write(outside.path().join("outside.txt"), "outside\n").expect("write outside path");
    assert!(matches!(
        adapter.commit_candidate(&run, &outside, &boundary()),
        Err(GitError::Containment(
            ContainmentViolation::OutsideMutable { .. }
        ))
    ));
    assert_head_unchanged(&outside);
    assert_eq!(repository.caller_state(), caller_before);
}

#[test]
fn unchanged_and_ignored_state_cannot_form_candidate_commits() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");

    let unchanged = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare unchanged candidate");
    assert!(matches!(
        adapter.commit_candidate(&run, &unchanged, &boundary()),
        Err(GitError::CandidateUnchanged)
    ));

    let ignored = adapter
        .prepare_candidate(&run, 2)
        .expect("prepare ignored candidate");
    fs::write(ignored.path().join("mutable/ignored.tmp"), "hidden\n").expect("write ignored path");
    assert!(matches!(
        adapter.commit_candidate(&run, &ignored, &boundary()),
        Err(GitError::IgnoredCandidatePaths { .. })
    ));
    assert_head_unchanged(&ignored);
}

#[test]
fn nested_repository_is_rejected_before_staging() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");
    let nested = candidate.path().join("mutable/nested");
    fs::create_dir(&nested).expect("create nested directory");
    git(&nested, &["init", "-q", "--initial-branch=main"]);

    assert!(matches!(
        adapter.commit_candidate(&run, &candidate, &boundary()),
        Err(GitError::NestedRepository { .. })
    ));
    assert_head_unchanged(&candidate);
    assert!(git_text(candidate.path(), &["diff", "--cached", "--name-only"]).is_empty());
}

#[cfg(unix)]
#[test]
fn symbolic_link_and_non_utf8_paths_are_rejected() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");

    let linked = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare linked candidate");
    symlink("/tmp", linked.path().join("mutable/escape")).expect("create candidate symlink");
    assert!(matches!(
        adapter.commit_candidate(&run, &linked, &boundary()),
        Err(GitError::CandidateSymlink { .. })
    ));
    assert_head_unchanged(&linked);

    let non_utf8 = adapter
        .prepare_candidate(&run, 2)
        .expect("prepare non-UTF-8 candidate");
    let blob = git_text(non_utf8.path(), &["rev-parse", "HEAD:mutable/edit.txt"]);
    let name = OsString::from_vec(b"mutable/bad\xff".to_vec());
    let output = Command::new("git")
        .arg("-C")
        .arg(non_utf8.path())
        .args([
            "update-index",
            "--add",
            "--cacheinfo",
            "100644",
            blob.as_str(),
        ])
        .arg(name)
        .output()
        .expect("stage non-UTF-8 path");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(matches!(
        adapter.commit_candidate(&run, &non_utf8, &boundary()),
        Err(GitError::NonUtf8Output { .. })
    ));
    assert_head_unchanged(&non_utf8);
}

#[test]
fn attached_foreign_stale_and_tampered_candidates_fail_closed() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");

    let foreign_run = RunWorkspace::new(
        autoresearch_core::RunId::new("run-2").expect("foreign run ID"),
        run.base_commit().clone(),
        run.head_commit().clone(),
    );
    let foreign = CandidateWorkspace::new(
        &foreign_run,
        1,
        &snapshot.root().join(".autoresearch/worktrees"),
    )
    .expect("foreign candidate");
    assert!(matches!(
        adapter.commit_candidate(&run, &foreign, &boundary()),
        Err(GitError::ForeignCandidateWorkspace)
    ));

    let attached = adapter
        .prepare_candidate(&run, 2)
        .expect("prepare attached candidate");
    fs::write(attached.path().join("mutable/edit.txt"), "attached\n")
        .expect("edit attached candidate");
    git(attached.path(), &["switch", "-qc", "hijack"]);
    assert!(matches!(
        adapter.commit_candidate(&run, &attached, &boundary()),
        Err(GitError::CandidateNotDetached { .. })
    ));

    let tampered = adapter
        .prepare_candidate(&run, 3)
        .expect("prepare tampered candidate");
    let stale = adapter
        .prepare_candidate(&run, 4)
        .expect("prepare stale candidate");
    fs::write(tampered.path().join("mutable/edit.txt"), "tampered\n")
        .expect("edit tampered candidate");
    let admin = candidate_admin_path(&tampered);
    fs::write(
        admin.join("gitdir"),
        snapshot.root().join(".git").to_string_lossy().as_bytes(),
    )
    .expect("tamper administrative backlink");
    assert!(matches!(
        adapter.commit_candidate(&run, &tampered, &boundary()),
        Err(GitError::CandidateRepositoryMismatch { .. })
    ));
    assert_head_unchanged(&tampered);

    fs::write(stale.path().join("mutable/edit.txt"), "stale\n").expect("edit stale candidate");
    git(&repository.0, &["update-ref", "-d", run.branch_ref()]);
    assert!(matches!(
        adapter.commit_candidate(&run, &stale, &boundary()),
        Err(GitError::StaleRunBranch { .. })
    ));
    assert_head_unchanged(&stale);
}

fn boundary() -> MutationBoundary {
    MutationBoundary::new(
        vec![path("mutable")],
        vec![path("mutable/protected"), path("program.md")],
    )
    .expect("valid boundary")
}

fn path(value: &str) -> RepoPath {
    RepoPath::new(value).expect("valid repository path")
}

fn commit_via_port<C>(
    committer: &C,
    run: &RunWorkspace,
    candidate: &CandidateWorkspace,
    boundary: &MutationBoundary,
) -> CandidateCommit
where
    C: CandidateCommitter,
    C::Error: Debug,
{
    committer
        .commit_candidate(run, candidate, boundary)
        .expect("commit through core port")
}

fn assert_head_unchanged(candidate: &CandidateWorkspace) {
    assert_eq!(
        git_text(candidate.path(), &["rev-parse", "HEAD"]),
        candidate.parent_commit().as_str()
    );
}

fn candidate_admin_path(candidate: &CandidateWorkspace) -> PathBuf {
    let contents = fs::read_to_string(candidate.path().join(".git")).expect("read Git marker");
    PathBuf::from(
        contents
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree marker"),
    )
}

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 fixture output")
        .trim()
        .to_owned()
}
