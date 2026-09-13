//! Integration coverage for retained branches and isolated worktrees.

use autoresearch_core::{CandidateWorkspace, RepositoryInspector, RunId, RunWorkspace};
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
fn baseline_worktree_is_exact_detached_reusable_and_never_moves_caller() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let before = (repository.branch(), repository.head(), repository.status());
    let lock = RunLockGuard::acquire(&snapshot, "run-baseline").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let baseline = adapter.prepare_baseline(&run).expect("prepare baseline");
    assert_eq!(
        adapter.prepare_baseline(&run).expect("reopen baseline"),
        baseline
    );
    assert_eq!(
        git_text(&baseline, &["rev-parse", "HEAD"]),
        snapshot.base_commit().as_str()
    );
    assert!(git_text(&baseline, &["branch", "--show-current"]).is_empty());
    assert_eq!(
        (repository.branch(), repository.head(), repository.status()),
        before
    );
    fs::write(baseline.join("tracked.txt"), "mutated\n").expect("dirty baseline");
    assert!(adapter.prepare_baseline(&run).is_err());
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
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    git(&repository.0, &["switch", "-q", "autoresearch/run-1"]);

    assert!(matches!(
        adapter.open_run(),
        Err(GitError::RunBranchCheckedOut { .. })
    ));
    assert!(matches!(
        adapter.prepare_candidate(&run, 1),
        Err(GitError::RunBranchCheckedOut { .. })
    ));
}

#[test]
fn prepare_candidate_creates_locked_detached_worktree_without_moving_caller() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let before = (repository.branch(), repository.head(), repository.status());
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");

    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");

    assert_eq!(candidate.worktree_id(), "candidate-000001");
    assert_eq!(candidate.parent_commit(), run.head_commit());
    assert_eq!(
        git_text(candidate.path(), &["branch", "--show-current"]),
        ""
    );
    assert_eq!(
        git_text(candidate.path(), &["rev-parse", "HEAD"]),
        repository.head()
    );
    assert!(git_text(candidate.path(), &["status", "--porcelain=v1"]).is_empty());
    let worktrees = git_text(&repository.0, &["worktree", "list", "--porcelain"]);
    assert!(worktrees.contains(candidate.path().to_string_lossy().as_ref()));
    assert!(worktrees.contains("locked autoresearch:run-1"));
    assert_eq!(
        (repository.branch(), repository.head(), repository.status()),
        before
    );
}

#[test]
fn prepare_candidate_refuses_existing_target_and_stale_run() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let existing = snapshot
        .root()
        .join(".autoresearch/worktrees/run-1/candidate-000001");
    fs::create_dir_all(&existing).expect("create collision");
    assert!(matches!(
        adapter.prepare_candidate(&run, 1),
        Err(GitError::WorktreePathExists(_))
    ));

    let tree = git_text(&repository.0, &["rev-parse", "HEAD^{tree}"]);
    let moved = git_text(
        &repository.0,
        &[
            "commit-tree",
            &tree,
            "-p",
            run.head_commit().as_str(),
            "-m",
            "move retained ref",
        ],
    );
    git(&repository.0, &["update-ref", run.branch_ref(), &moved]);
    assert!(matches!(
        adapter.prepare_candidate(&run, 2),
        Err(GitError::StaleRunBranch { .. })
    ));
}

#[test]
fn adapter_rejects_foreign_run_and_candidate_values() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let foreign_run = RunWorkspace::new(
        RunId::new("run-2").expect("foreign run ID"),
        run.base_commit().clone(),
        run.head_commit().clone(),
    );
    assert!(matches!(
        adapter.prepare_candidate(&foreign_run, 1),
        Err(GitError::ForeignRunWorkspace)
    ));

    let foreign_candidate = CandidateWorkspace::new(
        &foreign_run,
        1,
        &snapshot.root().join(".autoresearch/worktrees"),
    )
    .expect("foreign candidate value");
    assert!(matches!(
        adapter.retain_candidate(&run, &foreign_candidate),
        Err(GitError::ForeignCandidateWorkspace)
    ));
}

#[test]
fn retain_candidate_advances_only_run_ref() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let before = (repository.branch(), repository.head(), repository.status());
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");
    commit_candidate(&candidate, "candidate change\n", "candidate");
    let candidate_head = git_text(candidate.path(), &["rev-parse", "HEAD"]);

    let retained = adapter
        .retain_candidate(&run, &candidate)
        .expect("retain candidate");

    assert_eq!(retained.head_commit().as_str(), candidate_head);
    assert_eq!(run.head_commit(), snapshot.base_commit());
    assert_eq!(
        git_text(&repository.0, &["rev-parse", run.branch_ref()]),
        candidate_head
    );
    assert!(candidate.path().is_dir(), "cleanup belongs to later plan");
    assert_eq!(
        (repository.branch(), repository.head(), repository.status()),
        before
    );
}

#[test]
fn retain_candidate_rejects_dirty_attached_and_nonlinear_heads() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");

    let dirty = adapter.prepare_candidate(&run, 1).expect("dirty candidate");
    fs::write(dirty.path().join("untracked.txt"), "dirty\n").expect("dirty worktree");
    assert!(matches!(
        adapter.retain_candidate(&run, &dirty),
        Err(GitError::DirtyCandidate { .. })
    ));

    let attached = adapter
        .prepare_candidate(&run, 2)
        .expect("attached candidate");
    commit_candidate(&attached, "attached\n", "attached");
    git(attached.path(), &["switch", "-qc", "hijack"]);
    assert!(matches!(
        adapter.retain_candidate(&run, &attached),
        Err(GitError::CandidateNotDetached { .. })
    ));

    let nonlinear = adapter
        .prepare_candidate(&run, 3)
        .expect("nonlinear candidate");
    commit_candidate(&nonlinear, "first\n", "first candidate commit");
    commit_candidate(&nonlinear, "second\n", "second candidate commit");
    assert!(matches!(
        adapter.retain_candidate(&run, &nonlinear),
        Err(GitError::CandidateTopology { .. })
    ));
}

#[cfg(unix)]
#[test]
fn prepare_candidate_rejects_symlinked_worktree_parent() {
    use std::os::unix::fs::symlink;

    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let outside = snapshot.root().with_extension("worktrees-outside");
    fs::create_dir(&outside).expect("create outside directory");
    symlink(&outside, snapshot.root().join(".autoresearch/worktrees"))
        .expect("create worktree symlink");

    assert!(matches!(
        adapter.prepare_candidate(&run, 1),
        Err(GitError::UnsafeWorktreePath { .. })
    ));
    fs::remove_dir(outside).expect("remove outside directory");
}

fn commit_candidate(
    candidate: &autoresearch_core::CandidateWorkspace,
    contents: &str,
    message: &str,
) {
    fs::write(candidate.path().join("tracked.txt"), contents).expect("change candidate file");
    git(candidate.path(), &["add", "tracked.txt"]);
    git(candidate.path(), &["commit", "-qm", message]);
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
