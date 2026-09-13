//! Local bounded Candle evaluation through Phase 3 SDK and Phase 5 runner.

use autoresearch_config::{Evaluator, FrozenIdentity, ValidatedManifest};
use autoresearch_core::{Disposition, JournalEntry, JournalEvent, Measurement, NumericMetricKind};
use autoresearch_evaluator::{
    CancellationToken, EvaluationContext, EvaluatorFailure, FailureClass, NativeEvaluationError,
    NativeEvaluator, ValidatedOutput, evaluate_native,
};
use autoresearch_market::MarketEvidenceLedger;
use autoresearch_report::build_report;
use autoresearch_runner::{
    BaselineCapture, BaselineExecutor, RunMutationMode, RunStep, SubprocessBaselineExecutor,
    advance_run_once, capture_existing_baseline, load_report_source, verify_kept_commit,
};
use autoresearch_training_candle::TrainingEvaluator;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

const PROGRAM: &str = "Change only training-config.json within fixed budget.\n";
const GATE: &str = "Internal training fixture is not product demand evidence.\n";
const MANIFEST: &str = r#"
schema_version = 1
[experiment]
name = "tiny Candle training fixture"
[experiment.objective]
name = "val_bpb"
direction = "minimize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 60
[scope]
mutable_paths = ["training-config.json"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "candle_tiny_training"
hard_gates = ["tiny_fixture_verified"]
[evaluators.command]
program = "unused-native-adapter"
timeout_seconds = 40
[[evaluators.metrics]]
name = "val_bpb"
kind = "objective"
direction = "minimize"
[[evaluators.metrics]]
name = "parameter_count"
kind = "tie_breaker"
direction = "minimize"
[[evaluators.metrics]]
name = "runtime_millis"
kind = "diagnostic"
direction = "minimize"
[[evaluators.metrics]]
name = "training_tokens"
kind = "diagnostic"
direction = "maximize"
[[evaluators.metrics]]
name = "validation_tokens"
kind = "diagnostic"
direction = "maximize"
"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self::with_manifest(MANIFEST)
    }

    fn with_manifest(manifest_text: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autoresearch-training-runner-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("fixture root");
        git(&root, &["init", "-q", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "Training Fixture"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::create_dir(root.join("docs")).expect("docs");
        for (path, content) in [
            (".gitignore", ".autoresearch/\n"),
            (
                "training-config.json",
                "{\"steps\":4,\"max_wall_millis\":30000,\"learning_rate\":0.01}\n",
            ),
            ("autoresearch.toml", manifest_text),
            ("program.md", PROGRAM),
            ("docs/BET.md", GATE),
        ] {
            fs::write(root.join(path), content).expect("fixture file");
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        let base = git_text(&root, &["rev-parse", "HEAD"]);
        let manifest = ValidatedManifest::parse(manifest_text).expect("manifest");
        let identity = FrozenIdentity::capture(
            &manifest,
            PROGRAM.as_bytes(),
            &BTreeMap::new(),
            Some(GATE.as_bytes()),
        )
        .expect("identity");
        let run = root.join(".autoresearch/runs/run-1");
        fs::create_dir_all(run.join("frozen")).expect("frozen dir");
        for (path, content) in [
            ("autoresearch.toml", manifest_text),
            ("program.md", PROGRAM),
            ("product-gate.md", GATE),
        ] {
            fs::write(run.join("frozen").join(path), content).expect("frozen file");
        }
        fs::write(
            run.join("identity.json"),
            serde_json::to_vec(&identity).expect("identity JSON"),
        )
        .expect("identity file");
        let entry = JournalEntry {
            sequence: 0,
            run_id: "run-1".into(),
            event: JournalEvent::RunStarted {
                base_commit: base,
                frozen_identity: identity.aggregate_sha256,
            },
        };
        let mut bytes = serde_json::to_vec(&entry).expect("journal JSON");
        bytes.push(b'\n');
        fs::write(run.join("journal.jsonl"), bytes).expect("journal file");
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("owned fixture cleanup");
    }
}

struct Adapter(TrainingEvaluator);

impl BaselineExecutor for Adapter {
    fn evaluate(
        &self,
        context: &EvaluationContext,
        evaluator: &Evaluator,
    ) -> Result<ValidatedOutput, EvaluatorFailure> {
        if evaluator.id() != self.0.id() {
            return Err(EvaluatorFailure {
                class: FailureClass::Validation,
                detail: "wrong training evaluator ID".into(),
            });
        }
        evaluate_native(context, &self.0).map_err(|error| match error {
            NativeEvaluationError::Evaluator(failure) => failure,
            NativeEvaluationError::Output(_) => EvaluatorFailure {
                class: FailureClass::Validation,
                detail: "training output failed SDK validation".into(),
            },
        })
    }
}

#[test]
fn runner_captures_frozen_tiny_objective_and_run_owned_artifacts() {
    let fixture = Fixture::new();
    let before_head = git_text(&fixture.0, &["rev-parse", "HEAD"]);
    let before_status = git_text(&fixture.0, &["status", "--porcelain=v1"]);
    let result = capture_existing_baseline(
        &fixture.0,
        "run-1",
        &Adapter(TrainingEvaluator::default()),
        BTreeMap::new(),
    )
    .expect("runner baseline");
    let BaselineCapture::Captured { snapshot, .. } = result else {
        panic!("expected captured tiny objective")
    };
    assert_eq!(snapshot.measurements.len(), 6);
    assert!(snapshot.measurements.iter().any(|measurement| matches!(
        measurement,
        Measurement::Numeric { name, metric_kind: NumericMetricKind::Objective, .. } if name == "val_bpb"
    )));
    let artifacts = fixture.0.join(".autoresearch/runs/run-1/artifacts");
    let training_dir = fs::read_dir(artifacts)
        .expect("artifact root")
        .next()
        .expect("one artifact directory")
        .expect("artifact entry")
        .path();
    let evidence: serde_json::Value = serde_json::from_slice(
        &fs::read(training_dir.join("training-evidence.json")).expect("evidence file"),
    )
    .expect("evidence JSON");
    assert_eq!(evidence["evaluated_commit"], before_head);
    assert!(evidence["fixture_contract_sha256"].as_str().is_some());
    assert!(
        evidence["validation"]["objective"]["val_bpb"]
            .as_f64()
            .is_some()
    );
    assert!(evidence["validation"]["diagnostics"]["peak_memory_bytes"].is_null());
    assert!(training_dir.join("checkpoint-step-4.json").is_file());
    assert_eq!(git_text(&fixture.0, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(
        git_text(&fixture.0, &["status", "--porcelain=v1"]),
        before_status
    );
}

#[test]
fn runner_records_cancelled_failure_without_snapshot_or_score() {
    let fixture = Fixture::new();
    let cancel = Arc::new(AtomicBool::new(true));
    let result = capture_existing_baseline(
        &fixture.0,
        "run-1",
        &Adapter(TrainingEvaluator::with_cancellation(cancel)),
        BTreeMap::new(),
    )
    .expect("runner failure outcome");
    let BaselineCapture::Failed { failure, .. } = result else {
        panic!("cancelled training must not score")
    };
    assert_eq!(failure.class, FailureClass::Cancelled);
    let journal = fs::read_to_string(fixture.0.join(".autoresearch/runs/run-1/journal.jsonl"))
        .expect("journal");
    assert!(!journal.contains("baseline_captured"));
}

#[test]
fn subprocess_runner_captures_tiny_training_through_jsonl_contract() {
    let binary = env!("CARGO_BIN_EXE_autoresearch-training-evaluator");
    let manifest = MANIFEST.replace("unused-native-adapter", binary);
    let fixture = Fixture::with_manifest(&manifest);
    let executor = SubprocessBaselineExecutor::new(1_048_576, 16_384, CancellationToken::default())
        .expect("process caps");
    let result = capture_existing_baseline(&fixture.0, "run-1", &executor, BTreeMap::new())
        .expect("subprocess runner");
    let BaselineCapture::Captured { snapshot, .. } = result else {
        panic!("expected subprocess training objective")
    };
    assert_eq!(snapshot.measurements.len(), 6);
    assert!(snapshot.measurements.iter().any(|measurement| matches!(
        measurement,
        Measurement::Numeric { name, metric_kind: NumericMetricKind::Objective, .. } if name == "val_bpb"
    )));
}

#[test]
fn subprocess_runner_cancel_does_not_emit_score() {
    let binary = env!("CARGO_BIN_EXE_autoresearch-training-evaluator");
    let manifest = MANIFEST.replace("unused-native-adapter", binary);
    let fixture = Fixture::with_manifest(&manifest);
    let token = CancellationToken::default();
    token.cancel();
    let executor = SubprocessBaselineExecutor::new(1_048_576, 16_384, token).expect("process caps");
    let result = capture_existing_baseline(&fixture.0, "run-1", &executor, BTreeMap::new())
        .expect("subprocess failure outcome");
    let BaselineCapture::Failed { failure, .. } = result else {
        panic!("cancelled subprocess must not score")
    };
    assert_eq!(failure.class, FailureClass::Cancelled);
}

#[test]
fn disposable_candidate_produces_exact_decision_journal_and_report() {
    let binary = env!("CARGO_BIN_EXE_autoresearch-training-evaluator");
    let example = include_str!("../../../examples/nanochat-candle/autoresearch.toml");
    let manifest = example.replace("autoresearch-training-evaluator", binary);
    ValidatedManifest::parse(&manifest).expect("checked-in nanochat example manifest");
    let fixture = Fixture::with_manifest(&manifest);
    let caller_head = git_text(&fixture.0, &["rev-parse", "HEAD"]);
    let caller_config = fs::read(fixture.0.join("training-config.json")).expect("caller config");
    let executor = SubprocessBaselineExecutor::new(1_048_576, 16_384, CancellationToken::default())
        .expect("process caps");
    let baseline = capture_existing_baseline(&fixture.0, "run-1", &executor, BTreeMap::new())
        .expect("baseline");
    assert!(matches!(baseline, BaselineCapture::Captured { .. }));
    let prepared = advance_run_once(
        &fixture.0,
        "run-1",
        &RunMutationMode::Manual,
        None,
        &executor,
    )
    .expect("prepare candidate");
    let RunStep::AwaitingMutation { worktree, .. } = prepared else {
        panic!("isolated candidate expected")
    };
    fs::write(
        worktree.join("training-config.json"),
        b"{\"steps\":8,\"max_wall_millis\":30000,\"learning_rate\":0.01}\n",
    )
    .expect("bounded parameter mutation");
    let evaluated = advance_run_once(
        &fixture.0,
        "run-1",
        &RunMutationMode::Manual,
        Some("test eight fixed SGD steps"),
        &executor,
    )
    .expect("evaluate candidate");
    let RunStep::Evaluated(outcome) = evaluated else {
        panic!("candidate decision expected")
    };
    assert_eq!(
        git_text(&fixture.0, &["cat-file", "-t", &outcome.commit]),
        "commit"
    );
    assert_eq!(outcome.snapshot.measurements.len(), 6);
    assert_eq!(outcome.decision.disposition, Disposition::Keep);
    let verified =
        verify_kept_commit(&fixture.0, "run-1", &executor).expect("fresh kept-commit verification");
    assert_eq!(verified.verified_commit, outcome.commit);
    let fresh = verified
        .fresh_snapshot
        .as_ref()
        .expect("fresh evaluator evidence");
    let selected_bpb = numeric_value(&verified.selection_snapshot.measurements, "val_bpb");
    let fresh_bpb = numeric_value(&fresh.measurements, "val_bpb");
    assert!((selected_bpb - fresh_bpb).abs() <= 1.0e-6);
    assert!(verified.evidence_path.is_file());
    let journal = fs::read_to_string(fixture.0.join(".autoresearch/runs/run-1/journal.jsonl"))
        .expect("decision journal");
    assert!(journal.contains("candidate_decision_recorded"));
    assert!(journal.contains("candidate_finalized"));
    let source = load_report_source(&fixture.0, "run-1").expect("report source");
    let report = build_report(&source, MarketEvidenceLedger { receipts: vec![] })
        .expect("deterministic report");
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(
        report.candidates[0].candidate_commit.as_deref(),
        Some(outcome.commit.as_str())
    );
    assert_eq!(report.candidates[0].changed_paths, ["training-config.json"]);
    assert_eq!(report.candidates[0].artifacts.len(), 2);
    assert!(report.candidates[0].decision.is_some());
    assert!(report.candidates[0].finalization.is_some());
    assert_eq!(git_text(&fixture.0, &["rev-parse", "HEAD"]), caller_head);
    assert_eq!(
        fs::read(fixture.0.join("training-config.json")).expect("caller remains"),
        caller_config
    );
    assert_eq!(git_text(&fixture.0, &["status", "--porcelain=v1"]), "");
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git query");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git UTF-8")
        .trim()
        .into()
}

fn numeric_value(measurements: &[Measurement], name: &str) -> f64 {
    measurements
        .iter()
        .find_map(|measurement| match measurement {
            Measurement::Numeric {
                name: actual,
                value,
                ..
            } if actual == name => Some(value.get()),
            _ => None,
        })
        .expect("declared numeric metric")
}
