//! Declared-output, identity, ordering, and artifact-provenance tests.

use autoresearch_config::ValidatedManifest;
use autoresearch_core::{Complexity, Measurement, MetricDirection, NumericMetricKind};
use autoresearch_evaluator::{
    Artifact, ContextError, EvaluationContext, EvaluationContextSpec, EvaluatorOutput,
    NativeEvaluator, OutputError, PROTOCOL_VERSION, ProtocolResponse, ProtocolResult,
    ValidationError, build_snapshot, decode_response, encode_response, evaluate_native,
    validate_output,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const MANIFEST: &str = r#"
schema_version = 1
[experiment]
name = "output-validation"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 2
max_failures = 1
wall_clock_seconds = 30
[scope]
mutable_paths = ["src"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "quality"
hard_gates = ["tests", "lint"]
[evaluators.command]
program = "quality"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
[[evaluators.metrics]]
name = "elapsed"
kind = "diagnostic"
direction = "minimize"
[[evaluators]]
id = "safety"
hard_gates = ["contained"]
[evaluators.command]
program = "safety"
timeout_seconds = 10
"#;
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
            "autoresearch-evaluator-validation-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let candidate = root.join(".autoresearch/worktrees/run-1/candidate-000001");
        let artifacts = root.join(".autoresearch/runs/run-1/artifacts");
        fs::create_dir_all(&candidate).expect("candidate directory");
        fs::create_dir_all(&artifacts).expect("artifact directory");
        fs::write(artifacts.join("proof.txt"), "fixture evidence").expect("artifact fixture");
        Self {
            root,
            candidate,
            artifacts,
        }
    }

    fn context(&self) -> EvaluationContext {
        self.context_with_artifacts(self.artifacts.clone())
            .expect("valid context")
    }

    fn context_with_artifacts(
        &self,
        artifact_directory: PathBuf,
    ) -> Result<EvaluationContext, ContextError> {
        EvaluationContext::new(EvaluationContextSpec {
            run_id: "run-1".into(),
            baseline_commit: COMMIT.into(),
            evaluated_commit: COMMIT.into(),
            candidate_worktree: self.candidate.clone(),
            changed_paths: vec!["src/lib.rs".into()],
            declared_environment: BTreeMap::new(),
            artifact_directory,
            cancellation_id: "cancel-1".into(),
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn manifest() -> ValidatedManifest {
    ValidatedManifest::parse(MANIFEST).expect("valid manifest")
}

fn output(
    context: &EvaluationContext,
    evaluator_id: &str,
    measurements: Vec<Measurement>,
) -> EvaluatorOutput {
    EvaluatorOutput {
        evaluator_id: evaluator_id.into(),
        run_id: context.run_id().to_string(),
        baseline_commit: context.baseline_commit().to_string(),
        evaluated_commit: context.evaluated_commit().to_string(),
        measurements,
        observations: vec![],
        artifacts: vec![],
        warnings: vec![],
    }
}

fn quality(context: &EvaluationContext) -> EvaluatorOutput {
    let mut result = output(
        context,
        "quality",
        vec![
            Measurement::numeric(
                "elapsed",
                NumericMetricKind::Diagnostic,
                MetricDirection::Minimize,
                12.0,
            )
            .expect("metric"),
            Measurement::hard_gate("lint", true, None).expect("gate"),
            Measurement::numeric(
                "score",
                NumericMetricKind::Objective,
                MetricDirection::Maximize,
                0.9,
            )
            .expect("metric"),
            Measurement::hard_gate("tests", true, None).expect("gate"),
        ],
    );
    result.artifacts.push(Artifact {
        name: "proof".into(),
        relative_path: "proof.txt".into(),
        media_type: "text/plain".into(),
    });
    result
}

fn safety(context: &EvaluationContext) -> EvaluatorOutput {
    output(
        context,
        "safety",
        vec![Measurement::hard_gate("contained", true, None).expect("gate")],
    )
}

fn validated(
    context: &EvaluationContext,
    result: EvaluatorOutput,
) -> autoresearch_evaluator::ValidatedOutput {
    let id = result.evaluator_id.clone();
    validate_output(context, &id, result).expect("structural validation")
}

fn complete(
    context: &EvaluationContext,
    first: EvaluatorOutput,
    second: EvaluatorOutput,
) -> Result<autoresearch_core::EvaluationSnapshot, ValidationError> {
    build_snapshot(
        context,
        &manifest(),
        vec![validated(context, first), validated(context, second)],
        Complexity::default(),
    )
}

#[test]
fn preserves_manifest_evaluator_and_measurement_order() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let snapshot =
        complete(&context, safety(&context), quality(&context)).expect("complete output");
    let names = snapshot
        .measurements
        .iter()
        .map(Measurement::name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["tests", "lint", "score", "elapsed", "contained"]);
}

#[test]
fn rejects_missing_extra_and_duplicate_measurements() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut missing = quality(&context);
    missing
        .measurements
        .retain(|measurement| measurement.name() != "score");
    assert!(matches!(
        complete(&context, missing, safety(&context)),
        Err(ValidationError::MissingMeasurement(_))
    ));

    let mut missing_gate = quality(&context);
    missing_gate
        .measurements
        .retain(|measurement| measurement.name() != "lint");
    assert!(matches!(
        complete(&context, missing_gate, safety(&context)),
        Err(ValidationError::MissingMeasurement(_))
    ));

    let mut extra = quality(&context);
    extra
        .measurements
        .push(Measurement::hard_gate("surprise", true, None).expect("gate"));
    assert!(matches!(
        complete(&context, extra, safety(&context)),
        Err(ValidationError::ExtraMeasurement(_))
    ));

    let mut extra_metric = quality(&context);
    extra_metric.measurements.push(
        Measurement::numeric(
            "surprise_metric",
            NumericMetricKind::Diagnostic,
            MetricDirection::Maximize,
            1.0,
        )
        .expect("metric"),
    );
    assert!(matches!(
        complete(&context, extra_metric, safety(&context)),
        Err(ValidationError::ExtraMeasurement(_))
    ));

    let mut duplicate = quality(&context);
    duplicate
        .measurements
        .push(Measurement::hard_gate("lint", true, None).expect("gate"));
    assert!(matches!(
        validate_output(&context, "quality", duplicate),
        Err(OutputError::DuplicateMeasurement(_))
    ));

    let mut duplicate_metric = quality(&context);
    duplicate_metric.measurements.push(
        Measurement::numeric(
            "score",
            NumericMetricKind::Objective,
            MetricDirection::Maximize,
            0.9,
        )
        .expect("metric"),
    );
    assert!(matches!(
        validate_output(&context, "quality", duplicate_metric),
        Err(OutputError::DuplicateMeasurement(_))
    ));

    let only_quality = validated(&context, quality(&context));
    assert!(matches!(
        build_snapshot(
            &context,
            &manifest(),
            vec![only_quality],
            Complexity::default()
        ),
        Err(ValidationError::MissingEvaluator(_))
    ));
    let mut extra_evaluator = safety(&context);
    extra_evaluator.evaluator_id = "undeclared".into();
    assert!(matches!(
        complete(&context, quality(&context), extra_evaluator),
        Err(ValidationError::ExtraEvaluator(_))
    ));
}

#[test]
fn rejects_wrong_kind_direction_and_non_finite_values() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut wrong_kind = quality(&context);
    wrong_kind.measurements[0] = Measurement::hard_gate("elapsed", true, None).expect("gate");
    assert!(matches!(
        complete(&context, wrong_kind, safety(&context)),
        Err(ValidationError::WrongKind(_))
    ));

    let mut wrong_direction = quality(&context);
    wrong_direction.measurements[0] = Measurement::numeric(
        "elapsed",
        NumericMetricKind::Diagnostic,
        MetricDirection::Maximize,
        12.0,
    )
    .expect("metric");
    assert!(matches!(
        complete(&context, wrong_direction, safety(&context)),
        Err(ValidationError::WrongDirection(_))
    ));

    assert!(
        Measurement::numeric(
            "bad",
            NumericMetricKind::Diagnostic,
            MetricDirection::Maximize,
            f64::NAN
        )
        .is_err()
    );
    assert!(
        Measurement::numeric(
            "bad",
            NumericMetricKind::Diagnostic,
            MetricDirection::Maximize,
            f64::INFINITY
        )
        .is_err()
    );
    assert!(serde_json::from_str::<Measurement>(r#"{"kind":"numeric","name":"bad","metric_kind":"diagnostic","direction":"maximize","value":1e999}"#).is_err());
}

#[test]
fn rejects_identity_mismatch_and_sdk_market_evidence() {
    let fixture = Fixture::new();
    let context = fixture.context();
    for field in ["baseline", "candidate"] {
        let mut result = quality(&context);
        if field == "baseline" {
            result.baseline_commit = "a".repeat(40);
        } else {
            result.evaluated_commit = "b".repeat(40);
        }
        assert!(matches!(
            validate_output(&context, "quality", result),
            Err(OutputError::IdentityMismatch(_))
        ));
    }
    let mut result = quality(&context);
    result.measurements.push(
        Measurement::numeric(
            "receipts",
            NumericMetricKind::MarketEvidence,
            MetricDirection::Maximize,
            100.0,
        )
        .expect("metric"),
    );
    assert!(matches!(
        validate_output(&context, "quality", result),
        Err(OutputError::MarketEvidence(_))
    ));
}

#[test]
fn rejects_unsafe_missing_and_outside_artifacts() {
    let fixture = Fixture::new();
    let context = fixture.context();
    for unsafe_path in ["/tmp/outside.txt", "../outside.txt"] {
        let mut result = quality(&context);
        result.artifacts[0].relative_path = unsafe_path.into();
        assert!(matches!(
            validate_output(&context, "quality", result),
            Err(OutputError::UnsafeArtifactPath(_))
        ));
    }

    let mut missing = quality(&context);
    missing.artifacts[0].relative_path = "missing.txt".into();
    assert!(matches!(
        complete(&context, missing, safety(&context)),
        Err(ValidationError::ArtifactMissing(_))
    ));

    let other = fixture.root.join("other-artifacts");
    fs::create_dir_all(&other).expect("other artifact directory");
    let wrong_context = fixture
        .context_with_artifacts(other)
        .expect("context accepts canonical path");
    assert!(matches!(
        complete(
            &wrong_context,
            quality(&wrong_context),
            safety(&wrong_context)
        ),
        Err(ValidationError::ArtifactRoot(_))
    ));
}

#[cfg(unix)]
#[test]
fn rejects_symlink_artifact_escape() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let outside = fixture.root.join("outside.txt");
    fs::write(&outside, "outside").expect("outside fixture");
    symlink(&outside, fixture.artifacts.join("escape.txt")).expect("symlink fixture");
    let context = fixture.context();
    let mut result = quality(&context);
    result.artifacts[0].relative_path = "escape.txt".into();
    assert!(matches!(
        complete(&context, result, safety(&context)),
        Err(ValidationError::ArtifactEscape(_))
    ));
}

struct NativeQuality;

impl NativeEvaluator for NativeQuality {
    fn id(&self) -> &'static str {
        "quality"
    }

    fn evaluate(
        &self,
        context: &EvaluationContext,
    ) -> Result<EvaluatorOutput, autoresearch_evaluator::EvaluatorFailure> {
        Ok(quality(context))
    }
}

#[test]
fn native_and_process_envelopes_build_same_snapshot() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let native = evaluate_native(&context, &NativeQuality).expect("native validation");

    let wire = ProtocolResponse {
        protocol_version: PROTOCOL_VERSION,
        result: ProtocolResult::Success {
            output: quality(&context),
        },
    };
    let encoded = encode_response(&wire).expect("encode process envelope");
    let decoded = decode_response(&encoded).expect("decode process envelope");
    let ProtocolResult::Success { output } = decoded.result else {
        panic!("success fixture")
    };
    let process = validate_output(&context, "quality", output).expect("process validation");

    let native_snapshot = build_snapshot(
        &context,
        &manifest(),
        vec![native, validated(&context, safety(&context))],
        Complexity::default(),
    )
    .expect("native snapshot");
    let process_snapshot = build_snapshot(
        &context,
        &manifest(),
        vec![process, validated(&context, safety(&context))],
        Complexity::default(),
    )
    .expect("process snapshot");
    assert_eq!(native_snapshot, process_snapshot);
}
