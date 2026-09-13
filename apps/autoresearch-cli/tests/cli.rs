//! Process-level tests for safe baseline creation.

use autoresearch_core::{JournalEntry, RecoveryAction, replay_journal};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
static EVALUATOR: OnceLock<PathBuf> = OnceLock::new();
static MUTATOR: OnceLock<PathBuf> = OnceLock::new();
static IMPROVING_EVALUATOR: OnceLock<PathBuf> = OnceLock::new();

fn evaluator_fixture() -> &'static PathBuf {
    EVALUATOR.get_or_init(|| {
        let binary =
            std::env::temp_dir().join(format!("autoresearch-cli-evaluator-{}", std::process::id()));
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/autoresearch-evaluator/tests/fixtures/process_fixture.rs");
        let output = Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&binary)
            .arg(source)
            .output()
            .expect("compile Rust fixture");
        assert_success(&output);
        binary
    })
}

fn mutation_fixture() -> &'static PathBuf {
    MUTATOR.get_or_init(|| {
        let binary =
            std::env::temp_dir().join(format!("autoresearch-cli-mutator-{}", std::process::id()));
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mutation_fixture.rs");
        let output = Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&binary)
            .arg(source)
            .output()
            .expect("compile Rust mutation fixture");
        assert_success(&output);
        binary
    })
}

fn improving_evaluator_fixture() -> &'static PathBuf {
    IMPROVING_EVALUATOR.get_or_init(|| {
        let binary = std::env::temp_dir().join(format!(
            "autoresearch-cli-improving-evaluator-{}",
            std::process::id()
        ));
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/improving_evaluator.rs");
        let output = Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&binary)
            .arg(source)
            .output()
            .expect("compile Rust improving evaluator fixture");
        assert_success(&output);
        binary
    })
}

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
        git(&self.0, &["add", "-A"]);
        git(
            &self.0,
            &["commit", "-q", "-m", "test: add experiment contract"],
        );
    }

    fn configure_evaluator(&self) {
        let manifest_path = self.0.join("autoresearch.toml");
        let manifest = fs::read_to_string(&manifest_path).expect("read template manifest");
        let fixture = evaluator_fixture().display().to_string();
        let manifest = manifest
            .replace("name = \"changed_lines\"", "name = \"score\"")
            .replace("direction = \"minimize\"", "direction = \"maximize\"")
            .replace(
                "hard_gates = [\"repository_valid\"]",
                "hard_gates = [\"tests\"]",
            )
            .replace(
                "program = \"git\"\nargs = [\"diff\", \"--numstat\"]",
                &format!("program = \"{fixture}\"\nargs = [\"json-numeric\"]"),
            );
        fs::write(manifest_path, manifest).expect("configure fixture evaluator");
        fs::create_dir(self.0.join("src")).expect("create mutable root");
        fs::write(self.0.join("src/example.rs"), "pub fn baseline() {}\n").expect("write source");
    }

    fn configure_command_agent(&self) {
        let path = self.0.join("autoresearch.toml");
        let manifest = fs::read_to_string(&path).expect("read configured manifest");
        let agent = mutation_fixture().display().to_string();
        fs::write(
            path,
            manifest.replace(
                "program = \"git\"\nargs = [\"status\", \"--short\"]",
                &format!("program = \"{agent}\"\nargs = []"),
            ),
        )
        .expect("set local command agent");
    }

    fn configure_improving_evaluator(&self, executable: &Path) {
        let path = self.0.join("autoresearch.toml");
        let manifest = fs::read_to_string(&path).expect("read configured manifest");
        let original = evaluator_fixture().display().to_string();
        let replacement = executable.display().to_string();
        fs::write(
            path,
            manifest
                .replace(&original, &replacement)
                .replace("args = [\"json-numeric\"]", "args = []"),
        )
        .expect("set improving evaluator");
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
    repository.configure_evaluator();
    repository.commit_contract();

    let output = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse baseline report");
    assert_eq!(report["command"], "baseline");
    assert_eq!(report["next_action"], "run");
    assert_eq!(report["evidence_status"], "captured");
    assert!(report["snapshot"].is_object());
    let run_directory = PathBuf::from(
        report["run_directory"]
            .as_str()
            .expect("run directory string"),
    );
    assert!(run_directory.join("identity.json").is_file());
    assert!(run_directory.join("frozen/autoresearch.toml").is_file());
    assert!(run_directory.join("frozen/program.md").is_file());

    let journal = fs::read_to_string(run_directory.join("journal.jsonl")).expect("read journal");
    let entries: Vec<JournalEntry> = journal
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse journal entry"))
        .collect();
    assert_eq!(
        replay_journal(&entries)
            .expect("replay journal")
            .recovery_action(),
        RecoveryAction::PrepareCandidate {
            index: 1,
            parent_commit: report["base_commit"].as_str().expect("base commit").into(),
        }
    );
}

#[test]
fn report_replays_frozen_baseline_without_mutating_repository() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run ID");
    let run_dir = PathBuf::from(baseline["run_directory"].as_str().expect("run directory"));
    let journal_before = fs::read(run_dir.join("journal.jsonl")).expect("journal before");
    let report = run_cli(&repository.0, &["--json", "report", "--run-id", run_id]);
    assert_success(&report);
    let report: Value = serde_json::from_slice(&report.stdout).expect("report JSON");
    assert_eq!(report["command"], "report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["environment"]["status"], "captured");
    assert_eq!(report["baseline"]["commit"], baseline["base_commit"]);
    assert_eq!(report["current_best_commit"], baseline["base_commit"]);
    assert!(
        report["market_evidence"]["receipts"]
            .as_array()
            .expect("receipts")
            .is_empty()
    );
    assert_eq!(
        fs::read(run_dir.join("journal.jsonl")).expect("journal after"),
        journal_before
    );
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");
}

#[test]
fn baseline_reports_explicit_declared_evaluator_failure_without_score() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.commit_contract();
    let output = run_cli(&repository.0, &["--json", "baseline"]);
    assert_eq!(output.status.code(), Some(5));
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse failed baseline");
    assert_eq!(report["evidence_status"], "failed");
    assert!(report.get("snapshot").is_none());
    assert!(report["failure"].is_object());
}

#[test]
fn manual_run_is_bounded_and_keeps_caller_checkout_clean() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    let manifest_path = repository.0.join("autoresearch.toml");
    let manifest = fs::read_to_string(&manifest_path).expect("read configured manifest");
    fs::write(
        &manifest_path,
        manifest.replace("max_candidates = 8", "max_candidates = 1"),
    )
    .expect("bound run to one candidate");
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run id");
    let prepared = run_cli(&repository.0, &["--json", "run", "--run-id", run_id]);
    assert_success(&prepared);
    let prepared: Value = serde_json::from_slice(&prepared.stdout).expect("prepared JSON");
    assert_eq!(prepared["status"], "awaiting_mutation");
    let worktree = PathBuf::from(prepared["worktree"].as_str().expect("worktree"));
    assert!(
        worktree.starts_with(
            fs::canonicalize(&repository.0)
                .expect("canonical fixture repository")
                .join(".autoresearch/worktrees")
        )
    );
    assert!(worktree.join("src/example.rs").is_file());
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");

    fs::write(worktree.join("src/example.rs"), "pub fn changed() {}\n").expect("edit candidate");
    let result = run_cli(
        &repository.0,
        &[
            "--json",
            "run",
            "--run-id",
            run_id,
            "--hypothesis",
            "change one line",
        ],
    );
    assert_success(&result);
    let result: Value = serde_json::from_slice(&result.stdout).expect("candidate JSON");
    assert_eq!(result["status"], "evaluated");
    assert!(result["snapshot"].is_object());
    assert!(result["commit"].is_string());
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");
    assert_eq!(
        fs::read_to_string(repository.0.join("src/example.rs")).expect("caller source"),
        "pub fn baseline() {}\n"
    );
    let stopped = run_cli(&repository.0, &["--json", "run", "--run-id", run_id]);
    assert_success(&stopped);
    let stopped: Value = serde_json::from_slice(&stopped.stdout).expect("stopped JSON");
    assert_eq!(stopped["status"], "stopped");
    assert_eq!(stopped["stop_reason"], "candidate_limit");
}

#[test]
fn run_refuses_missing_authority_and_changed_protected_identity() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run id");
    let no_authority = run_cli(
        &repository.0,
        &["run", "--run-id", run_id, "--mode", "command"],
    );
    assert_eq!(no_authority.status.code(), Some(3));
    let wrong_executable = run_cli(
        &repository.0,
        &[
            "run",
            "--run-id",
            run_id,
            "--mode",
            "command",
            "--hypothesis",
            "change one line",
            "--allow-executable",
            "/usr/bin/true",
        ],
    );
    assert_eq!(wrong_executable.status.code(), Some(4));
    assert!(
        !repository
            .0
            .join(".autoresearch/worktrees")
            .join(run_id)
            .join("candidate-000001")
            .exists()
    );
    fs::write(repository.0.join("program.md"), "changed frozen program\n").expect("dirty program");
    let dirty = run_cli(&repository.0, &["run", "--run-id", run_id]);
    assert_eq!(dirty.status.code(), Some(4));
    assert!(
        !repository
            .0
            .join(".autoresearch/worktrees")
            .join(run_id)
            .join("candidate-000001")
            .exists()
    );
}

#[test]
fn allowlisted_command_mutates_only_isolated_candidate() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.configure_command_agent();
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run id");
    let executable = mutation_fixture().to_str().expect("binary path");
    let result = run_cli(
        &repository.0,
        &[
            "--json",
            "run",
            "--run-id",
            run_id,
            "--mode",
            "command",
            "--hypothesis",
            "one falsifiable change",
            "--allow-executable",
            executable,
        ],
    );
    assert_success(&result);
    let result: Value = serde_json::from_slice(&result.stdout).expect("command result");
    assert_eq!(result["status"], "evaluated");
    assert!(result["snapshot"].is_object());
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");
    assert_eq!(
        fs::read_to_string(repository.0.join("src/example.rs")).expect("caller source"),
        "pub fn baseline() {}\n"
    );
}

#[test]
fn run_refuses_committed_gate_or_program_change_after_freeze() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    fs::create_dir(repository.0.join("docs")).expect("create docs");
    fs::write(repository.0.join("docs/BET.md"), "# Frozen gate\n").expect("write gate");
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run id");
    fs::write(repository.0.join("docs/BET.md"), "# Moved gate\n").expect("change gate");
    git(&repository.0, &["add", "docs/BET.md"]);
    git(&repository.0, &["commit", "-q", "-m", "test: move gate"]);
    let changed = run_cli(&repository.0, &["run", "--run-id", run_id]);
    assert_eq!(changed.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&changed.stderr)
            .contains("caller HEAD differs from frozen base commit")
    );
    assert!(
        !repository
            .0
            .join(".autoresearch/worktrees")
            .join(run_id)
            .join("candidate-000001")
            .exists()
    );
}

#[test]
fn unbounded_manifest_budget_is_rejected_before_run_directory_creation() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    let path = repository.0.join("autoresearch.toml");
    let manifest = fs::read_to_string(&path).expect("template manifest");
    fs::write(
        path,
        manifest.replace("max_candidates = 8", "max_candidates = 0"),
    )
    .expect("write unbounded budget");
    repository.commit_contract();
    let output = run_cli(&repository.0, &["baseline"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(!repository.0.join(".autoresearch/runs").exists());
}

#[test]
fn status_is_read_only_and_resume_recovers_incomplete_baseline() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run ID");
    let journal_path = PathBuf::from(baseline["run_directory"].as_str().expect("run directory"))
        .join("journal.jsonl");
    let completed_journal = fs::read_to_string(&journal_path).expect("read completed journal");
    let first_line = completed_journal.lines().next().expect("run start");
    fs::write(&journal_path, format!("{first_line}\n")).expect("simulate interrupted baseline");
    let pending_journal = fs::read(&journal_path).expect("pending journal bytes");

    let status = run_cli(&repository.0, &["--json", "status", "--run-id", run_id]);
    assert_success(&status);
    let status: Value = serde_json::from_slice(&status.stdout).expect("status JSON");
    assert_eq!(status["recovery_action"], "capture_baseline");
    assert_eq!(
        fs::read(&journal_path).expect("unchanged journal"),
        pending_journal
    );
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");

    let resumed = run_cli(&repository.0, &["--json", "resume", "--run-id", run_id]);
    assert_success(&resumed);
    let resumed: Value = serde_json::from_slice(&resumed.stdout).expect("resume JSON");
    assert_eq!(resumed["status"], "baseline_captured");
    assert!(resumed["snapshot"].is_object());
    let ready = run_cli(&repository.0, &["--json", "resume", "--run-id", run_id]);
    assert_success(&ready);
    let ready: Value = serde_json::from_slice(&ready.stdout).expect("ready JSON");
    assert_eq!(ready["status"], "ready_for_candidate");
}

#[test]
fn stop_and_resume_report_cancelled_run_without_candidate() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run ID");
    let stopped = run_cli(&repository.0, &["--json", "stop", "--run-id", run_id]);
    assert_success(&stopped);
    let stopped: Value = serde_json::from_slice(&stopped.stdout).expect("stop JSON");
    assert_eq!(stopped["stop_reason"], "operator_cancelled");
    assert_eq!(stopped["recovery_action"], "finished");
    let resumed = run_cli(&repository.0, &["--json", "resume", "--run-id", run_id]);
    assert_success(&resumed);
    let resumed: Value = serde_json::from_slice(&resumed.stdout).expect("resume JSON");
    assert_eq!(resumed["status"], "finished");
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");
}

#[test]
fn resume_rejects_diverged_retained_ref() {
    let repository = TestRepository::new();
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run ID");
    let head = git_text(&repository.0, &["rev-parse", "HEAD"]);
    let tree = git_text(&repository.0, &["rev-parse", "HEAD^{tree}"]);
    let foreign = git_text(
        &repository.0,
        &[
            "commit-tree",
            &tree,
            "-p",
            &head,
            "-m",
            "foreign ref advance",
        ],
    );
    let branch = format!("refs/heads/autoresearch/{run_id}");
    git(&repository.0, &["update-ref", &branch, &foreign]);
    let result = run_cli(&repository.0, &["resume", "--run-id", run_id]);
    assert_eq!(result.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("retained run ref differs from journal current commit")
    );
}

#[test]
fn verify_kept_commit_writes_fresh_evidence_without_reselection() {
    let repository = TestRepository::new();
    let run_id = create_kept_run(&repository, improving_evaluator_fixture());
    let branch = format!("refs/heads/autoresearch/{run_id}");
    let kept_commit = git_text(&repository.0, &["rev-parse", &branch]);
    let journal = repository
        .0
        .join(".autoresearch/runs")
        .join(&run_id)
        .join("journal.jsonl");
    let selection_journal = fs::read(&journal).expect("read selection journal");

    let verified = run_cli(&repository.0, &["--json", "verify", "--run-id", &run_id]);
    assert!(
        verified.status.success(),
        "verify failed: stdout={} stderr={}",
        String::from_utf8_lossy(&verified.stdout),
        String::from_utf8_lossy(&verified.stderr)
    );
    let verified: Value = serde_json::from_slice(&verified.stdout).expect("verify JSON");
    assert_eq!(verified["status"], "matched");
    assert_eq!(verified["verified_commit"], kept_commit);
    assert!(verified["selection_snapshot"].is_object());
    assert!(verified["fresh_snapshot"].is_object());
    assert!(verified["fresh_snapshot"]["measurements"].is_array());
    assert!(verified["fresh_snapshot"].get("complexity").is_none());
    let evidence_path = PathBuf::from(verified["evidence_path"].as_str().expect("evidence path"));
    assert!(evidence_path.is_file());
    assert_eq!(
        fs::read(&journal).expect("journal unchanged"),
        selection_journal
    );
    assert_eq!(
        git_text(&repository.0, &["rev-parse", &branch]),
        kept_commit
    );
    assert_eq!(git_text(&repository.0, &["status", "--porcelain=v1"]), "");
}

#[test]
fn verify_reports_evaluator_unavailable_without_fabricating_fresh_score() {
    let repository = TestRepository::new();
    let local_binary = repository.0.join(".autoresearch/local-evaluator");
    fs::create_dir(repository.0.join(".autoresearch")).expect("create ignored state");
    fs::copy(improving_evaluator_fixture(), &local_binary).expect("copy isolated evaluator");
    let run_id = create_kept_run(&repository, &local_binary);
    fs::remove_file(&local_binary).expect("simulate evaluator unavailable");
    let verified = run_cli(&repository.0, &["--json", "verify", "--run-id", &run_id]);
    assert_eq!(verified.status.code(), Some(5));
    let verified: Value = serde_json::from_slice(&verified.stdout).expect("verify JSON");
    assert_eq!(verified["status"], "failed");
    assert!(verified["fresh_snapshot"].is_null());
    assert!(verified["failure"].is_object());
    assert!(PathBuf::from(verified["evidence_path"].as_str().expect("evidence path")).is_file());
}

fn create_kept_run(repository: &TestRepository, evaluator: &Path) -> String {
    assert_success(&run_cli(&repository.0, &["init"]));
    repository.configure_evaluator();
    repository.configure_improving_evaluator(evaluator);
    repository.commit_contract();
    let baseline = run_cli(&repository.0, &["--json", "baseline"]);
    assert_success(&baseline);
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON");
    let run_id = baseline["run_id"].as_str().expect("run ID");
    let prepared = run_cli(&repository.0, &["--json", "run", "--run-id", run_id]);
    assert_success(&prepared);
    let prepared: Value = serde_json::from_slice(&prepared.stdout).expect("prepared JSON");
    let worktree = PathBuf::from(prepared["worktree"].as_str().expect("worktree"));
    fs::write(worktree.join("src/example.rs"), "pub fn kept() {}\n").expect("edit candidate");
    let result = run_cli(
        &repository.0,
        &[
            "--json",
            "run",
            "--run-id",
            run_id,
            "--hypothesis",
            "improve frozen score",
        ],
    );
    assert_success(&result);
    let result: Value = serde_json::from_slice(&result.stdout).expect("candidate JSON");
    assert_eq!(result["finalization"]["outcome"], "kept");
    run_id.into()
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

fn git_text(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .expect("run Git fixture query");
    assert_success(&output);
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .into()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
