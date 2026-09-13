//! Integration coverage for read-only repository inspection.

use autoresearch_core::{RepoPath, RepositoryInspector};
use autoresearch_git::{GitError, GitRepository};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRepository(PathBuf);

impl TestRepository {
    fn new(ignore_run_state: bool) -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autoresearch-git-repository-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create fixture repository");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        if ignore_run_state {
            fs::write(root.join(".gitignore"), ".autoresearch/\n").expect("write gitignore");
        }
        fs::write(root.join("tracked.txt"), "baseline\n").expect("write tracked file");
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        Self(root)
    }

    fn head(&self) -> String {
        git_text(&self.0, &["rev-parse", "HEAD"])
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
fn inspects_clean_repository_without_modifying_caller_checkout() {
    let repository = TestRepository::new(true);
    let before = repository.status();
    let snapshot = GitRepository
        .inspect(&repository.0.join("."), "HEAD")
        .expect("inspect repository");

    assert_eq!(snapshot.root(), repository.0.canonicalize().expect("root"));
    assert_eq!(snapshot.base_commit().as_str(), repository.head());
    assert_eq!(snapshot.head_commit().as_str(), repository.head());
    assert_eq!(repository.status(), before);
}

#[test]
fn reads_only_exact_ordinary_commit_blob_with_size_cap() {
    let repository = TestRepository::new(true);
    let snapshot = GitRepository
        .inspect(&repository.0, "HEAD")
        .expect("inspect");
    let path = RepoPath::new("tracked.txt").expect("path");
    assert_eq!(
        GitRepository
            .read_blob_at_commit(&snapshot, &path, 9)
            .expect("read"),
        Some(b"baseline\n".to_vec())
    );
    assert!(matches!(
        GitRepository.read_blob_at_commit(&snapshot, &path, 1),
        Err(GitError::UnsafeSourceBlob { .. })
    ));
    assert_eq!(
        GitRepository
            .read_blob_at_commit(&snapshot, &RepoPath::new("absent.txt").expect("path"), 8)
            .expect("missing"),
        None
    );
}

#[test]
fn rejects_dirty_tracked_or_untracked_state() {
    let repository = TestRepository::new(true);
    fs::write(repository.0.join("untracked.txt"), "unsafe\n").expect("dirty fixture");

    let untracked_error = GitRepository
        .inspect(&repository.0, "HEAD")
        .expect_err("dirty repository must fail");
    assert!(matches!(untracked_error, GitError::DirtyRepository { .. }));

    fs::remove_file(repository.0.join("untracked.txt")).expect("remove untracked file");
    fs::write(repository.0.join("tracked.txt"), "changed\n").expect("modify tracked file");
    let tracked_error = GitRepository
        .inspect(&repository.0, "HEAD")
        .expect_err("modified tracked file must fail");
    assert!(matches!(tracked_error, GitError::DirtyRepository { .. }));
}

#[test]
fn rejects_missing_base_ref() {
    let repository = TestRepository::new(true);
    let error = GitRepository
        .inspect(&repository.0, "refs/heads/missing")
        .expect_err("missing base must fail");
    assert!(matches!(error, GitError::BaseRefNotFound { .. }));
}

#[test]
fn rejects_unignored_or_tracked_run_state() {
    let unignored = TestRepository::new(false);
    assert!(matches!(
        GitRepository.inspect(&unignored.0, "HEAD"),
        Err(GitError::RunStateNotIgnored)
    ));

    let tracked = TestRepository::new(true);
    fs::create_dir(tracked.0.join(".autoresearch")).expect("create run state");
    fs::write(tracked.0.join(".autoresearch/tracked"), "bad\n").expect("write run state");
    git(&tracked.0, &["add", "-f", ".autoresearch/tracked"]);
    git(&tracked.0, &["commit", "-qm", "track run state"]);
    assert!(matches!(
        GitRepository.inspect(&tracked.0, "HEAD"),
        Err(GitError::RunStateTracked { .. })
    ));
}

#[test]
fn rejects_bare_repository() {
    let repository = TestRepository::new(true);
    let bare = repository.0.with_extension("bare");
    let output = Command::new("git")
        .args(["clone", "--bare", "-q"])
        .arg(&repository.0)
        .arg(&bare)
        .output()
        .expect("clone bare fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(matches!(
        GitRepository.inspect(&bare, "HEAD"),
        Err(GitError::BareRepository)
    ));
    fs::remove_dir_all(bare).expect("remove bare fixture");
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
