//! Crash-boundary and hostile-state coverage for journal-driven Git recovery.

use autoresearch_core::{
    CandidateDecision, CandidateFinalization, CandidateRecovery, CandidateRecoveryManager,
    Complexity, DecisionReason, Disposition, EvaluationSnapshot, JournalEntry, JournalEvent,
    Measurement, MetricDirection, MutationBoundary, NumericMetricKind, RecoveredCandidateState,
    RepoPath, RepositoryInspector, RunView, replay_journal,
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
            "autoresearch-git-recovery-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create fixture repository");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::write(root.join(".gitignore"), ".autoresearch/\nignored.tmp\n")
            .expect("write gitignore");
        fs::create_dir(root.join("mutable")).expect("create mutable directory");
        fs::write(root.join("mutable/edit.txt"), "baseline\n").expect("write tracked file");
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
fn prepared_recovery_creates_resumes_and_classifies_candidate() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let caller_before = repository.caller_state();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let view = prepared_view(run.head_commit().as_str());

    let CandidateRecovery::EvaluationReady {
        candidate, state, ..
    } = recover_via_port(&adapter, &view).expect("recover missing candidate")
    else {
        panic!("expected evaluation recovery");
    };
    assert_eq!(state, RecoveredCandidateState::Prepared);
    assert!(candidate.path().is_dir());
    assert!(worktree_listing(&repository.0).contains("locked autoresearch:run-1"));

    fs::write(candidate.path().join("mutable/edit.txt"), "candidate\n").expect("edit candidate");
    let evidence = adapter
        .commit_candidate(&run, &candidate, &boundary())
        .expect("commit candidate");
    let CandidateRecovery::EvaluationReady {
        candidate: resumed,
        state: RecoveredCandidateState::Committed { commit },
        ..
    } = recover_via_port(&adapter, &view).expect("resume committed candidate")
    else {
        panic!("expected committed evaluation recovery");
    };
    assert_eq!(resumed, candidate);
    assert_eq!(&commit, evidence.commit_id());
    assert_eq!(repository.caller_state(), caller_before);
}

#[test]
fn keep_recovery_advances_cleans_and_repeats_without_guessing() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let caller_before = repository.caller_state();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");
    fs::write(candidate.path().join("mutable/edit.txt"), "kept\n").expect("edit candidate");
    let evidence = adapter
        .commit_candidate(&run, &candidate, &boundary())
        .expect("commit candidate");
    let mismatched = decided_view(
        run.head_commit().as_str(),
        "3333333333333333333333333333333333333333",
        Disposition::Keep,
    );
    assert!(matches!(
        recover_via_port(&adapter, &mismatched),
        Err(GitError::RecoveryStateConflict { .. })
    ));
    assert!(candidate.path().exists());
    assert_eq!(
        git_text(&repository.0, &["rev-parse", run.branch_ref()]),
        run.head_commit().as_str()
    );
    let view = decided_view(
        run.head_commit().as_str(),
        evidence.commit_id().as_str(),
        Disposition::Keep,
    );

    let first = recover_via_port(&adapter, &view).expect("finalize keep");
    assert_finalized_keep(&first, evidence.commit_id().as_str());
    assert!(!candidate.path().exists());
    assert!(!worktree_listing(&repository.0).contains(candidate.worktree_id()));
    assert_eq!(
        git_text(&repository.0, &["rev-parse", run.branch_ref()]),
        evidence.commit_id().as_str()
    );

    let repeated = recover_via_port(&adapter, &view).expect("repeat completed keep");
    assert_eq!(repeated, first);
    assert_eq!(repository.caller_state(), caller_before);
}

#[test]
fn keep_recovery_recognizes_ref_advanced_before_cleanup() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");
    fs::write(candidate.path().join("mutable/edit.txt"), "kept\n").expect("edit candidate");
    let evidence = adapter
        .commit_candidate(&run, &candidate, &boundary())
        .expect("commit candidate");
    adapter
        .retain_candidate(&run, &candidate)
        .expect("simulate completed ref side effect");
    let view = decided_view(
        run.head_commit().as_str(),
        evidence.commit_id().as_str(),
        Disposition::Keep,
    );

    let recovery = recover_via_port(&adapter, &view).expect("recover after ref advance");

    assert_finalized_keep(&recovery, evidence.commit_id().as_str());
    assert!(!candidate.path().exists());
}

#[test]
fn discard_recovery_finishes_after_unlock_and_is_idempotent() {
    let repository = TestRepository::new();
    let snapshot = repository.snapshot();
    let caller_before = repository.caller_state();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("acquire lock");
    let adapter = LockedGitRepository::new(&snapshot, &lock).expect("bind adapter");
    let run = adapter.open_run().expect("open run");
    let candidate = adapter
        .prepare_candidate(&run, 1)
        .expect("prepare candidate");
    fs::write(candidate.path().join("mutable/edit.txt"), "discarded\n").expect("edit candidate");
    let evidence = adapter
        .commit_candidate(&run, &candidate, &boundary())
        .expect("commit candidate");
    git_path(&repository.0, &["worktree", "unlock"], candidate.path());
    let view = decided_view(
        run.head_commit().as_str(),
        evidence.commit_id().as_str(),
        Disposition::Discard,
    );

    let first = recover_via_port(&adapter, &view).expect("finish unlocked discard");
    assert_eq!(
        first,
        CandidateRecovery::Finalized {
            run: run.clone(),
            outcome: CandidateFinalization::Discarded,
        }
    );
    assert!(!candidate.path().exists());
    let repeated = recover_via_port(&adapter, &view).expect("repeat completed discard");
    assert_eq!(repeated, first);
    assert_eq!(
        git_text(&repository.0, &["rev-parse", run.branch_ref()]),
        run.head_commit().as_str()
    );
    assert_eq!(repository.caller_state(), caller_before);
}

#[test]
fn dirty_foreign_lock_and_unregistered_collision_are_preserved() {
    let dirty_repository = TestRepository::new();
    let dirty_snapshot = dirty_repository.snapshot();
    let dirty_lock = RunLockGuard::acquire(&dirty_snapshot, "run-1").expect("acquire dirty lock");
    let dirty_adapter =
        LockedGitRepository::new(&dirty_snapshot, &dirty_lock).expect("bind dirty adapter");
    let dirty_run = dirty_adapter.open_run().expect("open dirty run");
    let dirty = dirty_adapter
        .prepare_candidate(&dirty_run, 1)
        .expect("prepare dirty candidate");
    fs::write(dirty.path().join("mutable/edit.txt"), "unfinished\n").expect("dirty candidate");
    let dirty_view = decided_view(
        dirty_run.head_commit().as_str(),
        dirty_run.head_commit().as_str(),
        Disposition::Discard,
    );
    assert!(matches!(
        recover_via_port(&dirty_adapter, &dirty_view),
        Err(GitError::DirtyCandidate { .. })
    ));
    assert!(dirty.path().exists(), "dirty evidence must remain");

    let locked_repository = TestRepository::new();
    let locked_snapshot = locked_repository.snapshot();
    let locked_lock =
        RunLockGuard::acquire(&locked_snapshot, "run-1").expect("acquire locked lock");
    let locked_adapter =
        LockedGitRepository::new(&locked_snapshot, &locked_lock).expect("bind locked adapter");
    let locked_run = locked_adapter.open_run().expect("open locked run");
    let locked = locked_adapter
        .prepare_candidate(&locked_run, 1)
        .expect("prepare locked candidate");
    git_path(&locked_repository.0, &["worktree", "unlock"], locked.path());
    git_path_with_reason(&locked_repository.0, locked.path(), "foreign-owner");
    let locked_view = decided_view(
        locked_run.head_commit().as_str(),
        locked_run.head_commit().as_str(),
        Disposition::Discard,
    );
    assert!(matches!(
        recover_via_port(&locked_adapter, &locked_view),
        Err(GitError::ForeignWorktreeLock { .. })
    ));
    assert!(
        locked.path().exists(),
        "foreign-locked worktree must remain"
    );

    let collision_repository = TestRepository::new();
    let collision_snapshot = collision_repository.snapshot();
    let collision_lock =
        RunLockGuard::acquire(&collision_snapshot, "run-1").expect("acquire collision lock");
    let collision_adapter = LockedGitRepository::new(&collision_snapshot, &collision_lock)
        .expect("bind collision adapter");
    let collision_run = collision_adapter.open_run().expect("open collision run");
    let collision = collision_adapter
        .prepare_candidate(&collision_run, 1)
        .expect("prepare collision candidate");
    git_path(
        &collision_repository.0,
        &["worktree", "unlock"],
        collision.path(),
    );
    git_path(
        &collision_repository.0,
        &["worktree", "remove"],
        collision.path(),
    );
    fs::create_dir_all(collision.path()).expect("create unregistered collision");
    fs::write(collision.path().join("sentinel"), "preserve\n").expect("write sentinel");
    let collision_view = decided_view(
        collision_run.head_commit().as_str(),
        collision_run.head_commit().as_str(),
        Disposition::Discard,
    );
    assert!(matches!(
        recover_via_port(&collision_adapter, &collision_view),
        Err(GitError::WorktreeRegistrationMismatch { .. })
    ));
    assert!(
        collision.path().join("sentinel").is_file(),
        "unregistered collision must remain"
    );
}

fn recover_via_port<M>(manager: &M, view: &RunView) -> Result<CandidateRecovery, M::Error>
where
    M: CandidateRecoveryManager,
    M::Error: Debug,
{
    manager
        .recover_candidate(view)?
        .ok_or_else(|| panic!("expected candidate recovery"))
}

fn assert_finalized_keep(recovery: &CandidateRecovery, expected_commit: &str) {
    let CandidateRecovery::Finalized { run, outcome } = recovery else {
        panic!("expected finalization");
    };
    assert_eq!(run.head_commit().as_str(), expected_commit);
    assert_eq!(
        outcome,
        &CandidateFinalization::Kept {
            commit: expected_commit.into(),
        }
    );
}

fn boundary() -> MutationBoundary {
    MutationBoundary::new(
        vec![RepoPath::new("mutable").expect("mutable path")],
        Vec::new(),
    )
    .expect("valid boundary")
}

fn prepared_view(base: &str) -> Box<RunView> {
    let mut entries = baseline_entries(base);
    entries.push(entry(
        2,
        JournalEvent::CandidatePrepared {
            index: 1,
            parent_commit: base.into(),
            worktree_id: "candidate-000001".into(),
        },
    ));
    run_view(&entries)
}

fn decided_view(base: &str, candidate_commit: &str, disposition: Disposition) -> Box<RunView> {
    let mut entries = baseline_entries(base);
    entries.push(entry(
        2,
        JournalEvent::CandidatePrepared {
            index: 1,
            parent_commit: base.into(),
            worktree_id: "candidate-000001".into(),
        },
    ));
    entries.push(entry(
        3,
        JournalEvent::CandidateDecisionRecorded {
            index: 1,
            candidate_commit: candidate_commit.into(),
            snapshot: snapshot(11.0),
            decision: CandidateDecision {
                disposition,
                reason: if disposition == Disposition::Keep {
                    DecisionReason::PrimaryImprovement
                } else {
                    DecisionReason::PrimaryRegression
                },
            },
        },
    ));
    run_view(&entries)
}

fn baseline_entries(base: &str) -> Vec<JournalEntry> {
    vec![
        entry(
            0,
            JournalEvent::RunStarted {
                base_commit: base.into(),
                frozen_identity: "frozen".into(),
            },
        ),
        entry(
            1,
            JournalEvent::BaselineCaptured {
                snapshot: snapshot(10.0),
            },
        ),
    ]
}

fn run_view(entries: &[JournalEntry]) -> Box<RunView> {
    let autoresearch_core::ReplayState::Run(view) = replay_journal(entries).expect("valid journal")
    else {
        panic!("expected run view");
    };
    view
}

fn entry(sequence: u64, event: JournalEvent) -> JournalEntry {
    JournalEntry {
        sequence,
        run_id: "run-1".into(),
        event,
    }
}

fn snapshot(value: f64) -> EvaluationSnapshot {
    EvaluationSnapshot {
        measurements: vec![
            Measurement::hard_gate("tests", true, None).expect("gate"),
            Measurement::numeric(
                "score",
                NumericMetricKind::Objective,
                MetricDirection::Maximize,
                value,
            )
            .expect("objective"),
        ],
        complexity: Complexity::default(),
    }
}

fn worktree_listing(repository: &Path) -> String {
    git_text(repository, &["worktree", "list", "--porcelain"])
}

fn git_path(repository: &Path, args: &[&str], path: &Path) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .arg(path)
        .output()
        .expect("run Git path fixture command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_path_with_reason(repository: &Path, path: &Path, reason: &str) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["worktree", "lock", "--reason", reason])
        .arg(path)
        .output()
        .expect("run Git lock fixture command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
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
