//! Disposable-repository baseline runner tests with validated fake evaluator.

use autoresearch_config::{Evaluator, FrozenIdentity, ValidatedManifest};
use autoresearch_core::{
    EvaluatorFailure, FailureClass, JournalEntry, JournalEvent, Measurement, MetricDirection,
    NumericMetricKind, ReplayState, replay_journal,
};
use autoresearch_evaluator::{
    EvaluationContext, EvaluatorOutput, ValidatedOutput, validate_output,
};
use autoresearch_runner::{
    BaselineCapture, BaselineExecutor, RunnerError, capture_existing_baseline,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const MANIFEST: &str = r#"
schema_version = 1
[experiment]
name = "baseline fixture"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 30
[scope]
mutable_paths = ["tracked.txt"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "one"
hard_gates = ["tests"]
[evaluators.command]
program = "fake-one"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
[[evaluators]]
id = "two"
hard_gates = ["lint"]
[evaluators.command]
program = "fake-two"
timeout_seconds = 10
"#;

struct Fixture {
    root: PathBuf,
    run_dir: PathBuf,
    base: String,
    frozen_identity: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "autoresearch-runner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("fixture root");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Autoresearch Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::create_dir(root.join("docs")).expect("docs");
        fs::write(root.join(".gitignore"), ".autoresearch/\n").expect("ignore state");
        fs::write(root.join("tracked.txt"), "baseline\n").expect("tracked");
        fs::write(root.join("autoresearch.toml"), MANIFEST).expect("manifest");
        fs::write(root.join("program.md"), "Improve only tracked.txt\n").expect("program");
        fs::write(root.join("docs/BET.md"), "Original gate stays fixed\n").expect("gate");
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        let base = git_text(&root, &["rev-parse", "HEAD"]);
        let run_dir = root.join(".autoresearch/runs/run-1");
        let frozen = run_dir.join("frozen");
        fs::create_dir_all(&frozen).expect("frozen directory");
        fs::write(frozen.join("autoresearch.toml"), MANIFEST).expect("frozen manifest");
        fs::write(frozen.join("program.md"), "Improve only tracked.txt\n").expect("frozen program");
        fs::write(
            frozen.join("product-gate.md"),
            "Original gate stays fixed\n",
        )
        .expect("frozen gate");
        let manifest = ValidatedManifest::parse(MANIFEST).expect("fixture manifest");
        let identity = FrozenIdentity::capture(
            &manifest,
            b"Improve only tracked.txt\n",
            &BTreeMap::new(),
            Some(b"Original gate stays fixed\n"),
        )
        .expect("identity");
        fs::write(
            run_dir.join("identity.json"),
            serde_json::to_vec(&identity).expect("identity JSON"),
        )
        .expect("identity artifact");
        let started = JournalEntry {
            sequence: 0,
            run_id: "run-1".into(),
            event: JournalEvent::RunStarted {
                base_commit: base.clone(),
                frozen_identity: identity.aggregate_sha256.clone(),
            },
        };
        let mut journal = serde_json::to_vec(&started).expect("journal JSON");
        journal.push(b'\n');
        fs::write(run_dir.join("journal.jsonl"), journal).expect("journal artifact");
        Self {
            root,
            run_dir,
            base,
            frozen_identity: identity.aggregate_sha256,
        }
    }

    fn journal(&self) -> Vec<JournalEntry> {
        fs::read_to_string(self.run_dir.join("journal.jsonl"))
            .expect("journal")
            .lines()
            .map(|line| serde_json::from_str(line).expect("entry"))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("owned fixture cleanup");
    }
}

struct FakeExecutor {
    called: RefCell<Vec<String>>,
    fail_on: Option<&'static str>,
    omit_metric: bool,
}

impl FakeExecutor {
    fn new(fail_on: Option<&'static str>, omit_metric: bool) -> Self {
        Self {
            called: RefCell::new(Vec::new()),
            fail_on,
            omit_metric,
        }
    }
}

impl BaselineExecutor for FakeExecutor {
    fn evaluate(
        &self,
        context: &EvaluationContext,
        evaluator: &Evaluator,
    ) -> Result<ValidatedOutput, EvaluatorFailure> {
        self.called.borrow_mut().push(evaluator.id().into());
        if self.fail_on == Some(evaluator.id()) {
            return Err(EvaluatorFailure {
                class: FailureClass::Timeout,
                detail: "internal raw diagnostic".into(),
            });
        }
        let mut measurements = vec![
            Measurement::hard_gate(
                if evaluator.id() == "one" {
                    "tests"
                } else {
                    "lint"
                },
                true,
                None,
            )
            .expect("gate"),
        ];
        if evaluator.id() == "one" && !self.omit_metric {
            measurements.push(
                Measurement::numeric(
                    "score",
                    NumericMetricKind::Objective,
                    MetricDirection::Maximize,
                    42.0,
                )
                .expect("score"),
            );
        }
        validate_output(
            context,
            evaluator.id(),
            EvaluatorOutput {
                evaluator_id: evaluator.id().into(),
                run_id: context.run_id().to_string(),
                baseline_commit: context.baseline_commit().to_string(),
                evaluated_commit: context.evaluated_commit().to_string(),
                measurements,
                observations: vec![],
                artifacts: vec![],
                warnings: vec![],
            },
        )
        .map_err(|_| EvaluatorFailure {
            class: FailureClass::Validation,
            detail: "fake output invalid".into(),
        })
    }
}

#[test]
fn baseline_uses_only_declared_evaluators_and_keeps_caller_checkout_unchanged() {
    let fixture = Fixture::new();
    let before = (
        git_text(&fixture.root, &["rev-parse", "HEAD"]),
        git_text(&fixture.root, &["branch", "--show-current"]),
        git_text(&fixture.root, &["status", "--porcelain=v1"]),
    );
    let executor = FakeExecutor::new(None, false);
    let result = capture_existing_baseline(&fixture.root, "run-1", &executor, BTreeMap::new())
        .expect("captured baseline");
    let BaselineCapture::Captured { snapshot, .. } = result else {
        panic!("expected baseline snapshot")
    };
    assert_eq!(snapshot.measurements.len(), 3);
    assert_eq!(*executor.called.borrow(), ["one", "two"]);
    assert_eq!(fixture.journal().len(), 2);
    let ReplayState::Run(view) = replay_journal(&fixture.journal()).expect("replay") else {
        panic!("run expected")
    };
    assert!(view.baseline().is_some());
    assert_eq!(view.frozen_identity(), fixture.frozen_identity);
    assert_eq!(view.base_commit(), fixture.base);
    assert_eq!(
        (
            git_text(&fixture.root, &["rev-parse", "HEAD"]),
            git_text(&fixture.root, &["branch", "--show-current"]),
            git_text(&fixture.root, &["status", "--porcelain=v1"])
        ),
        before
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("docs/BET.md")).expect("gate"),
        "Original gate stays fixed\n"
    );
}

#[test]
fn evaluator_failure_is_typed_terminal_and_never_reported_as_baseline_success() {
    let fixture = Fixture::new();
    let executor = FakeExecutor::new(Some("two"), false);
    let result = capture_existing_baseline(&fixture.root, "run-1", &executor, BTreeMap::new())
        .expect("recorded failure");
    let BaselineCapture::Failed {
        evaluator_id,
        failure,
        ..
    } = result
    else {
        panic!("failure expected")
    };
    assert_eq!(evaluator_id, "two");
    assert_eq!(failure.class, FailureClass::Timeout);
    assert!(!failure.detail.contains("raw diagnostic"));
    assert_eq!(*executor.called.borrow(), ["one", "two"]);
    let ReplayState::Run(view) = replay_journal(&fixture.journal()).expect("replay") else {
        panic!("run expected")
    };
    assert!(view.baseline().is_none());
    assert_eq!(view.baseline_failure().expect("typed failure").0, "two");
    assert!(capture_existing_baseline(&fixture.root, "run-1", &executor, BTreeMap::new()).is_err());
}

#[test]
fn incomplete_or_tampered_baseline_cannot_be_success() {
    let fixture = Fixture::new();
    let executor = FakeExecutor::new(None, false);
    fs::write(fixture.run_dir.join("frozen/program.md"), "changed\n").expect("tamper frozen input");
    assert!(matches!(
        capture_existing_baseline(&fixture.root, "run-1", &executor, BTreeMap::new()),
        Err(RunnerError::InvalidState(_))
    ));
    assert!(executor.called.borrow().is_empty());
    assert_eq!(fixture.journal().len(), 1);

    let fixture = Fixture::new();
    let incomplete = FakeExecutor::new(None, true);
    let result = capture_existing_baseline(&fixture.root, "run-1", &incomplete, BTreeMap::new())
        .expect("validation failure recorded");
    assert!(
        matches!(result, BaselineCapture::Failed { evaluator_id, failure, .. } if evaluator_id == "snapshot_validation" && failure.class == FailureClass::Validation)
    );
    let ReplayState::Run(view) = replay_journal(&fixture.journal()).expect("replay") else {
        panic!("run expected")
    };
    assert!(view.baseline().is_none());
}

#[test]
fn self_consistent_frozen_tamper_still_conflicts_with_original_commit() {
    let fixture = Fixture::new();
    let changed_program = b"Run unauthorized experiment\n";
    fs::write(fixture.run_dir.join("frozen/program.md"), changed_program).expect("tamper program");
    let manifest = ValidatedManifest::parse(MANIFEST).expect("manifest");
    let forged = FrozenIdentity::capture(
        &manifest,
        changed_program,
        &BTreeMap::new(),
        Some(b"Original gate stays fixed\n"),
    )
    .expect("forged identity");
    fs::write(
        fixture.run_dir.join("identity.json"),
        serde_json::to_vec(&forged).expect("identity JSON"),
    )
    .expect("forge stored identity");
    let started = JournalEntry {
        sequence: 0,
        run_id: "run-1".into(),
        event: JournalEvent::RunStarted {
            base_commit: fixture.base.clone(),
            frozen_identity: forged.aggregate_sha256,
        },
    };
    let mut journal = serde_json::to_vec(&started).expect("journal JSON");
    journal.push(b'\n');
    fs::write(fixture.run_dir.join("journal.jsonl"), journal).expect("forge journal");
    let executor = FakeExecutor::new(None, false);
    assert!(matches!(
        capture_existing_baseline(&fixture.root, "run-1", &executor, BTreeMap::new()),
        Err(RunnerError::InvalidState("frozen source differs from base commit"))
    ));
    assert!(executor.called.borrow().is_empty());
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
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .into()
}
