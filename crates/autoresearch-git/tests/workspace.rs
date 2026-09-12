//! Integration coverage for retained branches and isolated worktrees.

use autoresearch_core::RepositoryInspector;
use autoresearch_git::{GitError, GitRepository, LockedGitRepository, RunLockGuard};
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
            "autoresearch-git-workspace-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create fixture repository");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::write(root.join(".gitignore"), ".autoresearch/\n").expect("write gitignore");
        fs::write(root.join("tracked.txt"), "baseline\n").expect("write tracked file");
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        Self(root)
    }

    fn snapshot(&self) -> autoresearch_core::RepositorySnapshot {
        GitRepository
            .inspect(&self.0, "HEAD")
            .expect("inspect fixture")
    }

    fn head(&self) -> String {
        git_text(&self.0, &["rev-parse", "HEAD"])
    }

    fn branch(&self) -> String {
        git_text(&self.0, &["branch", "--show-current"])
    }

    fn status(&self) -> String {
        git_text(
            &self.0,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove fixture repository");
    }
}

#[test]
fn open_run_creates_idempotent_ref_without_moving_caller() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let before = (repository.branch(), repository.head(), repository.status());
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");

    let run = adapter.open_run().expect("create run branch");
    let resumed = adapter.open_run().expect("resume run branch");

    assert_eq!(run, resumed);
    assert_eq!(run.branch_ref(), "refs/heads/autoresearch/run-1");
    assert_eq!(run.base_commit(), snapshot.base_commit());
    assert_eq!(run.head_commit(), snapshot.base_commit());
    assert_eq!(
        git_text(&repository.0, &["rev-parse", run.branch_ref()]),
        snapshot.base_commit().as_str()
    );
    assert_eq!(
        (repository.branch(), repository.head(), repository.status()),
        before
    );
}

#[test]
fn open_run_resumes_descendant_but_rejects_divergence() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let branch_ref = "refs/heads/autoresearch/run-1";
    let tree = git_text(&repository.0, &["rev-parse", "HEAD^{tree}"]);
    let descendant = git_text(
        &repository.0,
        &[
            "commit-tree",
            &tree,
            "-p",
            snapshot.base_commit().as_str(),
            "-m",
            "descendant",
        ],
    );
    git(&repository.0, &["update-ref", branch_ref, &descendant]);
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    assert_eq!(
        adapter
            .open_run()
            .expect("resume descendant")
            .head_commit()
            .as_str(),
        descendant
    );
    drop(adapter);
    drop(lock);

    let unrelated = git_text(&repository.0, &["commit-tree", &tree, "-m", "unrelated"]);
    git(&repository.0, &["update-ref", branch_ref, &unrelated]);
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("reacquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    assert!(matches!(
        adapter.open_run(),
        Err(GitError::RunBranchDiverged { .. })
    ));
}

#[test]
fn adapter_rejects_lock_from_another_repository() {
    let first = TestRepository::new();
    let second = TestRepository::new();
    let first_snapshot = first.snapshot();
    let second_snapshot = second.snapshot();
    let lock = RunLockGuard::acquire(&first_snapshot, "run-1").expect("acquire lock");

    assert!(matches!(
        LockedGitRepository::new(&second_snapshot, &lock),
        Err(GitError::LockRepositoryMismatch)
    ));
}

#[test]
fn open_run_rejects_retained_branch_checked_out_in_caller() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let branch_ref = "refs/heads/autoresearch/run-1";
    git(
        &repository.0,
        &["update-ref", branch_ref, snapshot.base_commit().as_str()],
    );
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    git(&repository.0, &["switch", "-q", "autoresearch/run-1"]);
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");

    assert!(matches!(
        adapter.open_run(),
        Err(GitError::RunBranchCheckedOut { .. })
    ));
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
