//! Cargo format, Clippy, test, and build gate adapter fixtures.

use autoresearch_config::ValidatedManifest;
use autoresearch_evaluator::{
    CancellationToken, CargoCheck, EvaluationContext, EvaluationContextSpec, FailureClass,
    ProcessLimits, evaluate_cargo_check,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    candidate: PathBuf,
    artifacts: PathBuf,
}

impl Fixture {
    fn new(source: &str) -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).expect("canonical temp root");
        let root = base.join(format!(
            "autoresearch-cargo-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(candidate.join("src")).expect("candidate source directory");
        fs::create_dir_all(&artifacts).expect("artifact directory");
        fs::write(
            candidate.join("Cargo.toml"),
            "[package]\nname = \"cargo_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("manifest");
        fs::write(
            candidate.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"cargo_fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("lockfile");
        fs::write(candidate.join("src/lib.rs"), source).expect("source");
        Self {
            root,
            candidate,
            artifacts,
        }
    }

    fn context(&self) -> EvaluationContext {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let toolchain = PathBuf::from(&cargo).parent().map_or_else(
            || std::env::var("PATH").unwrap_or_default(),
            |path| format!("{}:/usr/bin:/bin", path.display()),
        );
        EvaluationContext::new(EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: COMMIT.into(),
            evaluated_commit: COMMIT.into(),
            candidate_worktree: self.candidate.clone(),
            changed_paths: vec!["src/lib.rs".into()],
            declared_environment: BTreeMap::from([
                ("PATH".into(), toolchain),
                ("CARGO_NET_OFFLINE".into(), "true".into()),
                (
                    "CARGO_TARGET_DIR".into(),
                    self.artifacts.join("target").to_string_lossy().into_owned(),
                ),
                ("RUSTC_WRAPPER".into(), String::new()),
            ]),
            artifact_directory: self.artifacts.clone(),
            cancellation_id: "cancel-1".into(),
        })
        .expect("valid context")
    }

    fn source(&self) -> Vec<u8> {
        fs::read(self.candidate.join("src/lib.rs")).expect("source")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn cargo_program() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn evaluator(
    check: CargoCheck,
    args: &[&str],
    gate: &str,
    program: &str,
) -> autoresearch_config::Evaluator {
    let args = args
        .iter()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r#"
schema_version=1
[experiment]
name="cargo-fixture"
[experiment.objective]
name="score"
direction="maximize"
[experiment.budget]
max_candidates=1
max_failures=1
wall_clock_seconds=120
[scope]
mutable_paths=["src"]
[agent]
program="manual"
timeout_seconds=1
[[evaluators]]
id="cargo-{}"
hard_gates=["{gate}"]
[evaluators.command]
program="{program}"
args=[{args}]
timeout_seconds=60
[[evaluators]]
id="score-source"
[evaluators.command]
program="unused"
timeout_seconds=1
[[evaluators.metrics]]
name="score"
kind="objective"
direction="maximize"
"#,
        check.label()
    );
    ValidatedManifest::parse(&source)
        .expect("valid manifest")
        .evaluators()[0]
        .clone()
}

fn limits() -> ProcessLimits {
    ProcessLimits::new(262_144, 262_144).expect("limits")
}

const PASS_SOURCE: &str = "pub fn answer() -> u32 {\n    42\n}\n";

#[test]
fn each_cargo_check_passes_with_distinct_declared_gate() {
    for check in [
        CargoCheck::Format,
        CargoCheck::Clippy,
        CargoCheck::Test,
        CargoCheck::Build,
    ] {
        let fixture = Fixture::new(PASS_SOURCE);
        let before = fixture.source();
        let context = fixture.context();
        let declared = evaluator(check, check.args(), check.gate_name(), &cargo_program());
        let result = evaluate_cargo_check(
            &context,
            &declared,
            check,
            limits(),
            &CancellationToken::default(),
        )
        .unwrap_or_else(|failure| panic!("{check:?} should pass: {failure:?}"));
        assert_eq!(result.output.measurements()[0].name(), check.gate_name());
        assert_eq!(
            fixture.source(),
            before,
            "candidate source must not be rewritten"
        );
    }
}

#[test]
fn each_cargo_check_failure_stays_failure() {
    for (check, source) in [
        (CargoCheck::Format, "pub fn answer()->u32{42}\n"),
        (
            CargoCheck::Clippy,
            "pub fn answer() -> u32 {\n    let unused = 1;\n    42\n}\n",
        ),
        (
            CargoCheck::Test,
            "pub fn answer() -> u32 {\n    42\n}\n#[cfg(test)] mod tests { #[test] fn fails() { assert!(false); } }\n",
        ),
        (
            CargoCheck::Build,
            "pub fn answer() -> u32 {\n    unknown_symbol\n}\n",
        ),
    ] {
        let fixture = Fixture::new(source);
        let before = fixture.source();
        let context = fixture.context();
        let declared = evaluator(check, check.args(), check.gate_name(), &cargo_program());
        let error = evaluate_cargo_check(
            &context,
            &declared,
            check,
            limits(),
            &CancellationToken::default(),
        )
        .expect_err("Cargo check must fail");
        assert_eq!(error.failure.class, FailureClass::NonZeroExit);
        assert_eq!(fixture.source(), before);
    }
}

#[test]
fn rejects_wrong_gate_args_program_or_offline_policy() {
    let fixture = Fixture::new(PASS_SOURCE);
    let context = fixture.context();
    let check = CargoCheck::Test;
    for declared in [
        evaluator(check, check.args(), "cargo_build", &cargo_program()),
        evaluator(
            check,
            &["test", "--target", "unsupported"],
            check.gate_name(),
            &cargo_program(),
        ),
        evaluator(check, check.args(), check.gate_name(), "/not/cargo"),
    ] {
        let error = evaluate_cargo_check(
            &context,
            &declared,
            check,
            limits(),
            &CancellationToken::default(),
        )
        .expect_err("unsupported config must not pass");
        assert_eq!(error.failure.class, FailureClass::Validation);
    }

    let mut spec = EvaluationContextSpec {
        run_id: "run-1".into(),
        baseline_commit: COMMIT.into(),
        evaluated_commit: COMMIT.into(),
        candidate_worktree: fixture.candidate.clone(),
        changed_paths: vec!["src/lib.rs".into()],
        declared_environment: BTreeMap::new(),
        artifact_directory: fixture.artifacts.clone(),
        cancellation_id: "cancel-1".into(),
    };
    spec.declared_environment
        .insert("CARGO_NET_OFFLINE".into(), "false".into());
    let online = EvaluationContext::new(spec).expect("context syntax valid");
    let declared = evaluator(check, check.args(), check.gate_name(), &cargo_program());
    let error = evaluate_cargo_check(
        &online,
        &declared,
        check,
        limits(),
        &CancellationToken::default(),
    )
    .expect_err("online Cargo must not run");
    assert_eq!(error.failure.class, FailureClass::Validation);
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_cargo_target_escape() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new(PASS_SOURCE);
    let before = fixture.source();
    symlink(&fixture.candidate, fixture.artifacts.join("target")).expect("fixture symlink");
    let context = fixture.context();
    let check = CargoCheck::Build;
    let declared = evaluator(check, check.args(), check.gate_name(), &cargo_program());
    let error = evaluate_cargo_check(
        &context,
        &declared,
        check,
        limits(),
        &CancellationToken::default(),
    )
    .expect_err("symlinked target must fail before Cargo starts");
    assert_eq!(error.failure.class, FailureClass::Validation);
    assert_eq!(fixture.source(), before);
}
