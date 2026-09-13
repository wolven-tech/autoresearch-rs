//! Minimal statically linked evaluator with shared SDK validation.

use autoresearch_core::Measurement;
use autoresearch_evaluator::{
    EvaluationContext, EvaluationContextSpec, EvaluatorFailure, EvaluatorOutput, FailureClass,
    NativeEvaluationError, NativeEvaluator, ValidatedOutput, evaluate_native,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const EXAMPLE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// Example evaluator linked directly into the caller binary.
pub struct SourcePresent;

impl NativeEvaluator for SourcePresent {
    fn id(&self) -> &'static str {
        "source-present"
    }

    fn evaluate(&self, context: &EvaluationContext) -> Result<EvaluatorOutput, EvaluatorFailure> {
        let present = context.candidate_worktree().join("src/lib.rs").is_file();
        let gate = Measurement::hard_gate("source_present", present, None).map_err(|_| {
            EvaluatorFailure {
                class: FailureClass::Validation,
                detail: "invalid example hard gate".into(),
            }
        })?;
        Ok(EvaluatorOutput {
            evaluator_id: self.id().into(),
            run_id: context.run_id().to_string(),
            baseline_commit: context.baseline_commit().to_string(),
            evaluated_commit: context.evaluated_commit().to_string(),
            measurements: vec![gate],
            observations: vec![],
            artifacts: vec![],
            warnings: vec![],
        })
    }
}

/// Calls native evaluator through shared structural validator.
///
/// # Errors
///
/// Returns native evaluator failure or shared structural validation failure.
pub fn evaluate_demo(
    context: &EvaluationContext,
) -> Result<ValidatedOutput, NativeEvaluationError> {
    evaluate_native(context, &SourcePresent)
}

fn main() -> Result<(), Box<dyn Error>> {
    let base = fs::canonicalize(std::env::temp_dir())?;
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = base.join(format!(
        "autoresearch-native-example-{}-{unique}",
        std::process::id()
    ));
    let candidate = root.join(".autoresearch/worktrees/example-run/candidate-000001");
    let artifacts = root.join(".autoresearch/runs/example-run/artifacts");
    fs::create_dir_all(candidate.join("src"))?;
    fs::create_dir_all(&artifacts)?;
    fs::write(candidate.join("src/lib.rs"), "pub fn ready() {}\n")?;
    let context = EvaluationContext::new(EvaluationContextSpec {
        run_id: "example-run".into(),
        baseline_commit: EXAMPLE_COMMIT.into(),
        evaluated_commit: EXAMPLE_COMMIT.into(),
        candidate_worktree: candidate,
        changed_paths: vec!["src/lib.rs".into()],
        declared_environment: BTreeMap::new(),
        artifact_directory: artifacts,
        cancellation_id: "example-cancel".into(),
    })?;
    let output = evaluate_demo(&context)?;
    println!("{}: {:?}", output.evaluator_id(), output.measurements());
    fs::remove_dir_all(root)?;
    Ok(())
}
