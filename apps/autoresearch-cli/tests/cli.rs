//! Process-level tests for safe baseline creation.

use autoresearch_core::{JournalEntry, RecoveryAction, replay_journal};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TestRepository(PathBuf);

impl TestRepository {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autoresearch-rs-cli-test-{}-{nanos}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create fixture repository");
        git(&path, &["init", "-q"]);
        git(&path, &["config", "user.name", "Autoresearch Test"]);
        git(
            &path,
            &["config", "user.email", "autoresearch@example.invalid"],
        );
        git(&path, &["config", "commit.gpgsign", "false"]);
        Self(path)
    }

    fn commit_contract(&self) {
        fs::write(self.0.join(".gitignore"), ".autoresearch/\n").expect("write ignore");
        git(
            &self.0,
            &["add", ".gitignore", "autoresearch.toml", "program.md"],
        );
        git(
            &self.0,
            &["commit", "-q", "-m", "test: add experiment contract"],
        );
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("autoresearch-rs-cli-test-"))
        {
            fs::remove_dir_all(&self.0).expect("remove fixture repository");
        }
    }
}

#[test]
fn baseline_freezes_clean_inputs_and_writes_replayable_start() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.commit_contract();

    let output = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse baseline report");
    assert_eq!(report["command"], "baseline");
    assert_eq!(report["next_action"], "capture_baseline");
    assert_eq!(report["evidence_status"], "pending evaluator evidence");
    let run_directory = PathBuf::from(
        report["run_directory"]
            .as_str()
            .expect("run directory string"),
    );
    assert!(run_directory.join("identity.json").is_file());
    assert!(run_directory.join("frozen/autoresearch.toml").is_file());
    assert!(run_directory.join("frozen/program.md").is_file());

    let journal = fs::read_to_string(run_directory.join("journal.jsonl")).expect("read journal");
    let entry: JournalEntry = serde_json::from_str(journal.trim()).expect("parse journal entry");
    assert_eq!(
        replay_journal(&[entry])
            .expect("replay journal")
            .recovery_action(),
        RecoveryAction::CaptureBaseline
    );
}

#[test]
fn baseline_refuses_dirty_frozen_input() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.commit_contract();
    fs::write(
        repository.0.join("program.md"),
        "changed after baseline approval\n",
    )
    .expect("dirty program");

    let output = run_cli(&repository.0, &["baseline"]);
    assert_eq!(output.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("frozen inputs have uncommitted changes")
    );
}

fn run_cli(repository: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_autoresearch"))
        .arg("--repository")
        .arg(repository)
        .args(args)
        .output()
        .expect("run autoresearch CLI")
}

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert_success(&output);
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
