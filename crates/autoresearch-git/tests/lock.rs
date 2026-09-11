//! Integration coverage for exclusive repository ownership.

use autoresearch_core::RepositoryInspector;
use autoresearch_git::{GitError, GitRepository, RunLockGuard, RunLockOwner};
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
            "autoresearch-git-lock-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create fixture repository");
        git(&root, &["init", "-q"]);
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
fn lock_records_owner_and_rejects_second_owner() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let guard = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");

    assert_eq!(guard.path(), snapshot.root().join(".autoresearch/run.lock"));
    assert_eq!(guard.owner().schema_version(), 1);
    assert_eq!(guard.owner().run_id(), "run-1");
    assert_eq!(guard.owner().process_id(), std::process::id());
    assert!(guard.owner().acquired_unix_millis() > 0);
    let stored: RunLockOwner =
        serde_json::from_slice(&fs::read(guard.path()).expect("read lock owner"))
            .expect("parse lock owner");
    assert_eq!(stored, *guard.owner());

    let error = RunLockGuard::acquire(&snapshot, "run-2").expect_err("second lock must fail");
    match error {
        GitError::LockHeld { detail, .. } => assert!(detail.contains("run-1")),
        other => panic!("unexpected error: {other}"),
    }
    assert!(repository.status().is_empty());
}

#[test]
fn dropped_guard_allows_reacquisition() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let first = RunLockGuard::acquire(&snapshot, "run-1").expect("first lock");
    drop(first);

    let second = RunLockGuard::acquire(&snapshot, "run-2").expect("reacquire lock");
    assert_eq!(second.owner().run_id(), "run-2");
    assert!(repository.status().is_empty());
}

#[test]
fn repositories_have_independent_locks() {
    let first_repository = TestRepository::new();
    let second_repository = TestRepository::new();
    let first = RunLockGuard::acquire(&first_repository.snapshot(), "run-a").expect("first lock");
    let second =
        RunLockGuard::acquire(&second_repository.snapshot(), "run-b").expect("second lock");

    assert_ne!(first.path(), second.path());
}

#[test]
fn rejects_snapshot_after_head_moves() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    fs::write(repository.0.join("tracked.txt"), "next\n").expect("change tracked file");
    git(&repository.0, &["add", "tracked.txt"]);
    git(&repository.0, &["commit", "-qm", "move head"]);

    assert!(matches!(
        RunLockGuard::acquire(&snapshot, "run-1"),
        Err(GitError::StaleSnapshot)
    ));
    let current = repository.snapshot();
    RunLockGuard::acquire(&current, "run-2").expect("stale lock released");
}

#[test]
fn rejects_invalid_run_ids() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    assert!(matches!(
        RunLockGuard::acquire(&snapshot, ""),
        Err(GitError::InvalidRunId)
    ));
    assert!(matches!(
        RunLockGuard::acquire(&snapshot, "run/escape"),
        Err(GitError::InvalidRunId)
    ));
}

#[cfg(unix)]
#[test]
fn rejects_symlink_lock_file() {
    use std::os::unix::fs::symlink;

    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let state = repository.0.join(".autoresearch");
    fs::create_dir(&state).expect("create state");
    let outside = repository.0.join("outside-lock");
    fs::write(&outside, "do not overwrite\n").expect("write outside target");
    symlink(&outside, state.join("run.lock")).expect("create lock symlink");

    assert!(matches!(
        RunLockGuard::acquire(&snapshot, "run-1"),
        Err(GitError::UnsafeRunState { .. })
    ));
    assert_eq!(
        fs::read_to_string(outside).expect("read outside target"),
        "do not overwrite\n"
    );
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
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("UTF-8 fixture output")
        .trim()
        .to_owned()
}
