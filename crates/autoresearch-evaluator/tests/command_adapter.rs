//! Generic local command adapter tests using disposable Rust executable.

use autoresearch_config::ValidatedManifest;
use autoresearch_core::{Measurement, NumericMetricKind};
use autoresearch_evaluator::{
    CancellationToken, EvaluationContext, EvaluationContextSpec, FailureClass, ProcessLimits,
    evaluate_command_gate, evaluate_subprocess,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

fn fixture_executable() -> &'static PathBuf {
    EXECUTABLE.get_or_init(|| {
        let output = fs::canonicalize(std::env::temp_dir())
            .expect("temp root")
            .join(format!(
                "autoresearch-command-fixture-{}",
                std::process::id()
            ));
        fs::create_dir_all(&output).expect("fixture build directory");
        let binary = output.join("evaluator-fixture");
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process_fixture.rs");
        let result = Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&binary)
            .arg(source)
            .output()
            .expect("launch fixture compiler");
        assert!(
            result.status.success(),
            "fixture compiler: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        binary
    })
}

struct Fixture {
    root: PathBuf,
    candidate: PathBuf,
    artifacts: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).expect("canonical temp root");
        let root = base.join(format!(
            "autoresearch-command-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate directory");
        fs::create_dir_all(&artifacts).expect("artifact directory");
        Self {
            root,
            candidate,
            artifacts,
        }
    }

    fn context(&self, declared_environment: BTreeMap<String, String>) -> EvaluationContext {
        EvaluationContext::new(EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: COMMIT.into(),
            evaluated_commit: COMMIT.into(),
            candidate_worktree: self.candidate.clone(),
            changed_paths: vec!["src/lib.rs".into()],
            declared_environment,
            artifact_directory: self.artifacts.clone(),
            cancellation_id: "cancel-1".into(),
        })
        .expect("valid context")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn evaluator(
    mode: &str,
    numeric: bool,
    gates: &[&str],
    timeout: u64,
) -> autoresearch_config::Evaluator {
    let program = fixture_executable().to_str().expect("UTF-8 path");
    let gates = gates
        .iter()
        .map(|gate| format!("\"{gate}\""))
        .collect::<Vec<_>>()
        .join(",");
    let metric = if numeric {
        "[[evaluators.metrics]]\nname=\"score\"\nkind=\"objective\"\ndirection=\"maximize\""
    } else {
        ""
    };
    let other = if numeric {
        ""
    } else {
        r#"
[[evaluators]]
id="score-source"
[evaluators.command]
program="unused"
timeout_seconds=1
[[evaluators.metrics]]
name="score"
kind="objective"
direction="maximize"
"#
    };
    let source = format!(
        r#"
schema_version=1
[experiment]
name="command-fixture"
[experiment.objective]
name="score"
direction="maximize"
[experiment.budget]
max_candidates=1
max_failures=1
wall_clock_seconds=10
[scope]
mutable_paths=["src"]
[agent]
program="manual"
timeout_seconds=1
[[evaluators]]
id="command"
hard_gates=[{gates}]
[evaluators.command]
program="{program}"
args=["{mode}"]
timeout_seconds={timeout}
{metric}
{other}
"#
    );
    ValidatedManifest::parse(&source)
        .expect("fixture manifest")
        .evaluators()[0]
        .clone()
}

fn limits() -> ProcessLimits {
    ProcessLimits::new(4096, 4096).expect("limits")
}

#[test]
fn raw_command_success_and_extra_stdout_create_only_declared_gate() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    for mode in ["raw-success", "raw-extra", "raw-cwd"] {
        let declared = evaluator(mode, false, &["tests"], 2);
        let result =
            evaluate_command_gate(&context, &declared, limits(), &CancellationToken::default())
                .expect("command pass");
        assert_eq!(result.output.measurements().len(), 1);
        assert!(
            matches!(&result.output.measurements()[0],Measurement::HardGate{name,outcome} if name=="tests" && outcome.passed())
        );
        assert!(result.diagnostics.stdout_bytes > 0);
    }
}

#[test]
fn raw_command_failure_and_timeout_never_fabricate_score() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    for (mode, timeout, class) in [
        ("raw-fail", 2, FailureClass::NonZeroExit),
        ("hang", 1, FailureClass::Timeout),
    ] {
        let declared = evaluator(mode, false, &["tests"], timeout);
        let error =
            evaluate_command_gate(&context, &declared, limits(), &CancellationToken::default())
                .expect_err("command fails");
        assert_eq!(error.failure.class, class);
        assert!(!error.failure.detail.contains("do-not-leak"));
    }
}

#[test]
fn raw_command_receives_only_declared_environment() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::from([("ALLOW_ME".into(), "ok".into())]));
    let declared = evaluator("raw-env", false, &["tests"], 2);
    evaluate_command_gate(&context, &declared, limits(), &CancellationToken::default())
        .expect("declared env only");
}

#[test]
fn raw_mode_rejects_numeric_or_ambiguous_gate_declarations() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    for declared in [
        evaluator("raw-success", true, &["tests"], 2),
        evaluator("raw-success", false, &["tests", "lint"], 2),
    ] {
        let error =
            evaluate_command_gate(&context, &declared, limits(), &CancellationToken::default())
                .expect_err("raw mode declaration rejected");
        assert_eq!(error.failure.class, FailureClass::Validation);
    }
}

#[test]
fn numeric_score_requires_versioned_jsonl_response() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    let raw = evaluator("raw-success", true, &["tests"], 2);
    let error = evaluate_subprocess(&context, &raw, limits(), &CancellationToken::default())
        .expect_err("raw text is not numeric evidence");
    assert_eq!(error.failure.class, FailureClass::Protocol);

    let jsonl = evaluator("json-numeric", true, &["tests"], 2);
    let result = evaluate_subprocess(&context, &jsonl, limits(), &CancellationToken::default())
        .expect("versioned numeric result");
    assert!(matches!(
        &result.output.measurements()[1],
        Measurement::Numeric {
            metric_kind: NumericMetricKind::Objective,
            ..
        }
    ));
}
