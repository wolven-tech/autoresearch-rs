//! Manual mutation stays inside prepared worktree and frozen Git boundary.

use autoresearch_config::{FrozenIdentity, ValidatedManifest};
use autoresearch_core::RepositoryInspector;
use autoresearch_git::{GitError, GitRepository, LockedGitRepository, RunLockGuard};
use autoresearch_runner::{MutationError, MutationRequest, submit_manual_candidate};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const MANIFEST: &str = r#"
schema_version = 1
[experiment]
name = "manual fixture"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 30
[scope]
mutable_paths = ["tracked.txt", "allowed.txt"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "one"
hard_gates = ["tests"]
[evaluators.command]
program = "fake"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "autoresearch-manual-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("root");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::create_dir(root.join("docs")).expect("docs");
        for (path, content) in [
            (".gitignore", ".autoresearch/\n"),
            ("tracked.txt", "initial\n"),
            ("allowed.txt", "initial\n"),
            ("outside.txt", "initial\n"),
            ("autoresearch.toml", MANIFEST),
            ("program.md", "Improve score within scope\n"),
            ("docs/BET.md", "Original gate\n"),
        ] {
            fs::write(root.join(path), content).expect("write fixture");
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        Self(root)
    }

    fn manifest() -> ValidatedManifest {
        ValidatedManifest::parse(MANIFEST).expect("manifest")
    }

    fn identity() -> FrozenIdentity {
        FrozenIdentity::capture(
            &Self::manifest(),
            b"Improve score within scope\n",
            &BTreeMap::new(),
            Some(b"Original gate\n"),
        )
        .expect("identity")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("fixture cleanup");
    }
}

#[test]
fn manual_mode_commits_only_contained_candidate_and_preserves_caller() {
    let fixture = Fixture::new();
    let snapshot = GitRepository.inspect(&fixture.0, "HEAD").expect("snapshot");
    let caller_head = snapshot.head_commit().to_string();
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("lock");
    let git = LockedGitRepository::new(&snapshot, &lock).expect("locked Git");
    let run = git.open_run().expect("run");
    let candidate = git.prepare_candidate(&run, 1).expect("candidate");
    let request = MutationRequest::new(
        &candidate,
        &Fixture::manifest(),
        "Improve score within scope\n",
        &Fixture::identity(),
        vec![],
        "Change tracked source to improve score",
        "run-1-candidate-1",
    )
    .expect("request");
    assert_eq!(request.candidate_worktree(), candidate.path());
    assert_eq!(request.allowed_files().len(), 2);
    assert_eq!(request.frozen_rubric().hard_gates, ["tests"]);
    assert_eq!(request.frozen_rubric().objective, "score");
    let payload = serde_json::to_value(&request).expect("provider-neutral JSON");
    assert!(payload.get("candidate_worktree").is_some());
    assert!(payload.get("allowed_files").is_some());
    assert!(payload.get("frozen_rubric").is_some());
    assert!(payload.get("prior_decisions").is_some());
    assert!(payload.get("hypothesis").is_some());
    assert!(payload.get("cancellation_id").is_some());
    assert!(payload.get("boundary").is_none());
    fs::write(candidate.path().join("tracked.txt"), "candidate\n").expect("manual edit");
    let committed = submit_manual_candidate(&fixture.0, &git, &run, &candidate, &request)
        .expect("contained commit");
    assert_eq!(committed.changed_paths().len(), 1);
    assert_eq!(committed.changed_paths()[0].as_str(), "tracked.txt");
    assert_eq!(git_text(&fixture.0, &["rev-parse", "HEAD"]), caller_head);
    assert_eq!(
        fs::read_to_string(fixture.0.join("tracked.txt")).expect("caller source"),
        "initial\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("docs/BET.md")).expect("gate"),
        "Original gate\n"
    );
}

#[test]
fn request_requires_hypothesis_and_manual_commit_rejects_unlisted_and_frozen_files() {
    let fixture = Fixture::new();
    let snapshot = GitRepository.inspect(&fixture.0, "HEAD").expect("snapshot");
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("lock");
    let git = LockedGitRepository::new(&snapshot, &lock).expect("locked Git");
    let run = git.open_run().expect("run");
    let candidate = git.prepare_candidate(&run, 1).expect("candidate");
    assert!(matches!(
        MutationRequest::new(
            &candidate,
            &Fixture::manifest(),
            "Improve score within scope",
            &Fixture::identity(),
            vec![],
            "  ",
            "run-1-candidate-1"
        ),
        Err(MutationError::MissingHypothesis)
    ));
    assert!(matches!(
        MutationRequest::new(
            &candidate,
            &Fixture::manifest(),
            "Edited frozen prompt",
            &Fixture::identity(),
            vec![],
            "Test prompt identity",
            "run-1-candidate-1"
        ),
        Err(MutationError::InvalidIdentity)
    ));
    let request = MutationRequest::new(
        &candidate,
        &Fixture::manifest(),
        "Improve score within scope\n",
        &Fixture::identity(),
        vec![],
        "Test bounded edit",
        "run-1-candidate-1",
    )
    .expect("request");
    fs::write(candidate.path().join("outside.txt"), "changed\n").expect("outside edit");
    assert!(matches!(
        submit_manual_candidate(&fixture.0, &git, &run, &candidate, &request),
        Err(MutationError::Git(GitError::Containment(_)))
    ));
    fs::write(candidate.path().join("outside.txt"), "initial\n").expect("restore outside");
    fs::write(candidate.path().join("program.md"), "changed program\n").expect("frozen edit");
    assert!(matches!(
        submit_manual_candidate(&fixture.0, &git, &run, &candidate, &request),
        Err(MutationError::Git(GitError::Containment(_)))
    ));
}

#[cfg(unix)]
#[test]
fn manual_mode_rejects_symlink_and_changed_caller_checkout() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let snapshot = GitRepository.inspect(&fixture.0, "HEAD").expect("snapshot");
    let lock = RunLockGuard::acquire(&snapshot, "run-1").expect("lock");
    let git = LockedGitRepository::new(&snapshot, &lock).expect("locked Git");
    let run = git.open_run().expect("run");
    let candidate = git.prepare_candidate(&run, 1).expect("candidate");
    let request = MutationRequest::new(
        &candidate,
        &Fixture::manifest(),
        "Improve score within scope\n",
        &Fixture::identity(),
        vec![],
        "Test symlink containment",
        "run-1-candidate-1",
    )
    .expect("request");
    fs::remove_file(candidate.path().join("allowed.txt")).expect("remove allowed");
    symlink(
        fixture.0.join("outside.txt"),
        candidate.path().join("allowed.txt"),
    )
    .expect("symlink escape");
    assert!(matches!(
        submit_manual_candidate(&fixture.0, &git, &run, &candidate, &request),
        Err(MutationError::Git(GitError::CandidateSymlink { .. }))
    ));
    fs::remove_file(candidate.path().join("allowed.txt")).expect("remove symlink");
    fs::write(candidate.path().join("allowed.txt"), "candidate\n").expect("valid edit");
    fs::write(fixture.0.join("tracked.txt"), "caller changed\n").expect("caller edit");
    assert!(matches!(
        submit_manual_candidate(&fixture.0, &git, &run, &candidate, &request),
        Err(MutationError::CallerChanged)
    ));
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .into()
}
