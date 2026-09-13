//! Contract and safety tests for evaluator invocation contexts.

use autoresearch_core::Measurement;
use autoresearch_evaluator::{
    ContextError, EvaluationContext, EvaluationContextSpec, EvaluatorOutput, NativeEvaluator,
    OutputError, evaluate_native, validate_output,
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
    fn new() -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).expect("canonical temp root");
        let root = base.join(format!(
            "autoresearch-evaluator-context-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let candidate = root.join("candidate");
        let artifacts = root.join("artifacts");
        fs::create_dir_all(&candidate).expect("candidate directory");
        fs::create_dir_all(&artifacts).expect("artifact directory");
        Self {
            root,
            candidate,
            artifacts,
        }
    }

    fn spec(&self) -> EvaluationContextSpec {
        EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: COMMIT.into(),
            evaluated_commit: COMMIT.into(),
            candidate_worktree: self.candidate.clone(),
            changed_paths: vec!["src/lib.rs".into()],
            declared_environment: BTreeMap::from([("CARGO_TERM_COLOR".into(), "never".into())]),
            artifact_directory: self.artifacts.clone(),
            cancellation_id: "cancel-1".into(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

#[test]
fn valid_context_preserves_exact_identity_and_paths() {
    let fixture = Fixture::new();
    let context = EvaluationContext::new(fixture.spec()).expect("valid context");
    assert_eq!(context.run_id().as_str(), "run-1");
    assert_eq!(context.baseline_commit().as_str(), COMMIT);
    assert_eq!(context.evaluated_commit().as_str(), COMMIT);
    assert_eq!(context.candidate_worktree(), fixture.candidate);
    assert_eq!(context.changed_paths()[0].as_str(), "src/lib.rs");
    assert_eq!(context.declared_environment()["CARGO_TERM_COLOR"], "never");
    assert_eq!(context.artifact_directory(), fixture.artifacts);
    assert_eq!(context.cancellation_id().as_str(), "cancel-1");
}

#[test]
fn rejects_invalid_ids_and_duplicate_or_escaping_changes() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec.run_id = " ".into();
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::RunId(_))
    ));

    let mut spec = fixture.spec();
    spec.baseline_commit = "short".into();
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::BaselineCommit(_))
    ));

    let mut spec = fixture.spec();
    spec.evaluated_commit = "not-a-commit".into();
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::EvaluatedCommit(_))
    ));

    let mut spec = fixture.spec();
    spec.changed_paths = vec!["src/lib.rs".into(), "src/lib.rs".into()];
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::DuplicateChangedPath(_))
    ));

    let mut spec = fixture.spec();
    spec.changed_paths = vec!["../outside".into()];
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::ChangedPath(_))
    ));
}

#[test]
fn rejects_unsafe_candidate_and_artifact_directories() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec.candidate_worktree = PathBuf::from("relative");
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::CandidateWorktree(_))
    ));

    let mut spec = fixture.spec();
    spec.artifact_directory = PathBuf::from("/");
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::ArtifactDirectory(_))
    ));

    let mut spec = fixture.spec();
    spec.artifact_directory = fixture.candidate.clone();
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::OverlappingDirectories)
    ));
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_candidate_directory() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let link = fixture.root.join("candidate-link");
    symlink(&fixture.candidate, &link).expect("create fixture symlink");
    let mut spec = fixture.spec();
    spec.candidate_worktree = link;
    assert!(matches!(
        EvaluationContext::new(spec),
        Err(ContextError::CandidateWorktree(_))
    ));
}

struct ConstantEvaluator;

impl NativeEvaluator for ConstantEvaluator {
    fn id(&self) -> &'static str {
        "constant"
    }

    fn evaluate(
        &self,
        context: &EvaluationContext,
    ) -> Result<EvaluatorOutput, autoresearch_evaluator::EvaluatorFailure> {
        Ok(EvaluatorOutput {
            evaluator_id: self.id().into(),
            run_id: context.run_id().to_string(),
            baseline_commit: context.baseline_commit().to_string(),
            evaluated_commit: context.evaluated_commit().to_string(),
            measurements: vec![Measurement::hard_gate("tests", true, None).expect("valid gate")],
            observations: vec![],
            artifacts: vec![],
            warnings: vec![],
        })
    }
}

#[test]
fn native_output_uses_common_validator_before_consumption() {
    let fixture = Fixture::new();
    let context = EvaluationContext::new(fixture.spec()).expect("valid context");
    let evaluator = ConstantEvaluator;
    let validated = evaluate_native(&context, &evaluator).expect("valid native output");
    assert_eq!(validated.measurements().len(), 1);

    let serialized = serde_json::to_vec(&evaluator.evaluate(&context).expect("native result"))
        .expect("serialize external envelope fixture");
    let process_envelope: EvaluatorOutput =
        serde_json::from_slice(&serialized).expect("deserialize external envelope fixture");
    let process_validated =
        validate_output(&context, evaluator.id(), process_envelope).expect("valid process output");
    assert_eq!(validated, process_validated);

    let mut wrong = evaluator.evaluate(&context).expect("native result");
    wrong.evaluated_commit = "a".repeat(40);
    assert!(matches!(
        validate_output(&context, evaluator.id(), wrong),
        Err(OutputError::IdentityMismatch(_))
    ));
}
