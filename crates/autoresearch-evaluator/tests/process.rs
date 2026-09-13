//! Bounded subprocess evaluator and environment-isolation tests.

use autoresearch_config::ValidatedManifest;
use autoresearch_evaluator::{
    CancellationToken, EvaluationContext, EvaluationContextSpec, FailureClass, ProcessLimits,
    evaluate_subprocess,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

fn fixture_executable() -> &'static PathBuf {
    EXECUTABLE.get_or_init(|| {
        let output = fs::canonicalize(std::env::temp_dir())
            .expect("temp root")
            .join(format!(
                "autoresearch-process-fixture-{}",
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
            .expect("launch rustc fixture compiler");
        assert!(
            result.status.success(),
            "fixture compiler failed: {}",
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
            "autoresearch-process-test-{}-{}",
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

    fn context(&self, environment: BTreeMap<String, String>) -> EvaluationContext {
        EvaluationContext::new(EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: COMMIT.into(),
            evaluated_commit: COMMIT.into(),
            candidate_worktree: self.candidate.clone(),
            changed_paths: vec!["src/lib.rs".into()],
            declared_environment: environment,
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
    program: &str,
    mode: &str,
    literal: Option<&str>,
    timeout: u64,
) -> autoresearch_config::Evaluator {
    let args = if let Some(literal) = literal {
        format!("\"{mode}\", \"{literal}\"")
    } else {
        format!("\"{mode}\"")
    };
    let source = format!(
        r#"
schema_version = 1
[experiment]
name = "process-test"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 10
[scope]
mutable_paths = ["src"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "fixture"
hard_gates = ["tests"]
[evaluators.command]
program = "{program}"
args = [{args}]
timeout_seconds = {timeout}
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
"#
    );
    ValidatedManifest::parse(&source)
        .expect("fixture manifest")
        .evaluators()[0]
        .clone()
}

fn limits() -> ProcessLimits {
    ProcessLimits::new(4096, 4096).expect("bounded limits")
}

#[test]
fn success_and_stderr_only_diagnostics_return_validated_output() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    for mode in ["success", "stderr"] {
        let evaluator = evaluator(
            fixture_executable().to_str().expect("UTF-8 path"),
            mode,
            None,
            2,
        );
        let result = evaluate_subprocess(
            &context,
            &evaluator,
            limits(),
            &CancellationToken::default(),
        )
        .expect("valid process output");
        assert_eq!(result.output.measurements().len(), 1);
        assert!(result.diagnostics.stdout_bytes > 0);
        assert!(
            !result
                .diagnostics
                .redacted_stderr
                .contains("safe diagnostic")
        );
    }
}

#[test]
fn classifies_nonzero_spawn_and_malformed_stdout_without_scores() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    for (program, mode, class) in [
        (
            fixture_executable().to_str().expect("UTF-8 path"),
            "nonzero",
            FailureClass::NonZeroExit,
        ),
        (
            "/definitely/not/a/real/evaluator",
            "success",
            FailureClass::Spawn,
        ),
        (
            fixture_executable().to_str().expect("UTF-8 path"),
            "malformed",
            FailureClass::Protocol,
        ),
    ] {
        let evaluator = evaluator(program, mode, None, 2);
        let error = evaluate_subprocess(
            &context,
            &evaluator,
            limits(),
            &CancellationToken::default(),
        )
        .expect_err("process must fail");
        assert_eq!(error.failure.class, class);
        assert!(!error.failure.detail.contains("do-not-leak"));
        assert!(!error.diagnostics.redacted_stderr.contains("do-not-leak"));
    }
}

#[test]
fn timeout_and_cancellation_terminate_owned_child() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    let evaluator = evaluator(
        fixture_executable().to_str().expect("UTF-8 path"),
        "hang",
        None,
        1,
    );
    let timed_out = evaluate_subprocess(
        &context,
        &evaluator,
        limits(),
        &CancellationToken::default(),
    )
    .expect_err("deadline must stop fixture");
    assert_eq!(timed_out.failure.class, FailureClass::Timeout);

    let token = CancellationToken::default();
    let to_cancel = token.clone();
    let cancellation = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        to_cancel.cancel();
    });
    let cancelled = evaluate_subprocess(&context, &evaluator, limits(), &token)
        .expect_err("cancellation must stop fixture");
    cancellation.join().expect("cancellation helper");
    assert_eq!(cancelled.failure.class, FailureClass::Cancelled);
}

#[test]
fn bounds_stdout_and_stderr_independently() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::new());
    for mode in ["stdout-overflow", "stderr-overflow"] {
        let evaluator = evaluator(
            fixture_executable().to_str().expect("UTF-8 path"),
            mode,
            None,
            2,
        );
        let error = evaluate_subprocess(
            &context,
            &evaluator,
            ProcessLimits::new(128, 128).expect("limits"),
            &CancellationToken::default(),
        )
        .expect_err("output must be bounded");
        assert_eq!(error.failure.class, FailureClass::OutputLimit);
    }
}

#[test]
fn scrubbed_environment_and_literal_arguments_are_enforced() {
    let fixture = Fixture::new();
    let context = fixture.context(BTreeMap::from([("ALLOW_ME".into(), "ok".into())]));
    let env_evaluator = evaluator(
        fixture_executable().to_str().expect("UTF-8 path"),
        "env",
        None,
        2,
    );
    evaluate_subprocess(
        &context,
        &env_evaluator,
        limits(),
        &CancellationToken::default(),
    )
    .expect("only declared environment reaches child");

    let evaluator = evaluator(
        fixture_executable().to_str().expect("UTF-8 path"),
        "literal",
        Some("$(touch /tmp/never-run)"),
        2,
    );
    evaluate_subprocess(
        &context,
        &evaluator,
        limits(),
        &CancellationToken::default(),
    )
    .expect("metacharacters remain literal argument");
}
