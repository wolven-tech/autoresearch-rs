//! Phase 3 native evaluator adapter for frozen tiny CPU training.

use crate::{
    CheckpointStore, MeasureFailure, SgdState, TinyCorpus, TinyGpt, TinyModelConfig, TrainConfig,
    TrainFailure, environment_fingerprint, measure_validation, sha256, train_fixed_budget,
};
use autoresearch_core::{Measurement, MetricDirection, NumericMetricKind};
use autoresearch_evaluator::{
    Artifact, EvaluationContext, EvaluatorFailure, EvaluatorOutput, FailureClass, NativeEvaluator,
    Warning,
};
use candle_core::Device;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_CONFIG_BYTES: u64 = 4 * 1024;
/// Frozen ID to declare in manifest.
pub const TRAINING_EVALUATOR_ID: &str = "candle_tiny_training";

/// Native adapter; CLI subprocess wrapper can use same implementation.
#[derive(Debug, Clone)]
pub struct TrainingEvaluator {
    cancelled: Arc<AtomicBool>,
}

impl Default for TrainingEvaluator {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl TrainingEvaluator {
    /// Constructs adapter sharing caller cancellation flag.
    #[must_use]
    pub fn with_cancellation(cancelled: Arc<AtomicBool>) -> Self {
        Self { cancelled }
    }
}

#[derive(Serialize)]
struct BoundEvidence<'a> {
    schema_version: u8,
    run_id: &'a str,
    baseline_commit: &'a str,
    evaluated_commit: &'a str,
    fixture_contract_sha256: &'a str,
    config: TrainConfig,
    validation: crate::ValidationEvidence,
}

impl NativeEvaluator for TrainingEvaluator {
    fn id(&self) -> &str {
        TRAINING_EVALUATOR_ID
    }

    fn evaluate(&self, context: &EvaluationContext) -> Result<EvaluatorOutput, EvaluatorFailure> {
        evaluate_training(context, &self.cancelled)
    }
}

fn evaluate_training(
    context: &EvaluationContext,
    cancelled: &AtomicBool,
) -> Result<EvaluatorOutput, EvaluatorFailure> {
    let path = context.candidate_worktree().join("training-config.json");
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| failure(FailureClass::Validation, "training config missing"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err(failure(
            FailureClass::Validation,
            "training config unsafe or oversized",
        ));
    }
    let bytes = fs::read(path)
        .map_err(|_| failure(FailureClass::Validation, "training config unreadable"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CONFIG_BYTES {
        return Err(failure(
            FailureClass::Validation,
            "training config oversized",
        ));
    }
    let config: TrainConfig = serde_json::from_slice(&bytes)
        .map_err(|_| failure(FailureClass::Validation, "training config invalid"))?;
    let corpus = TinyCorpus::embedded()
        .map_err(|_| failure(FailureClass::Validation, "frozen training fixture invalid"))?;
    let model = TinyGpt::new(TinyModelConfig::for_corpus(&corpus), &Device::Cpu)
        .map_err(|_| failure(FailureClass::Reported, "training model unavailable"))?;
    let fingerprint = environment_fingerprint(&model, &corpus)
        .map_err(|_| failure(FailureClass::Validation, "training environment unavailable"))?;
    let started = Instant::now();
    let training = train_fixed_budget(&model, &corpus, config, cancelled)
        .map_err(|error| map_train_error(&error))?;
    let validation = measure_validation(&model, &corpus, &training, started, &fingerprint)
        .map_err(|error| map_measure_error(&error))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            failure(
                FailureClass::Reported,
                "training artifact clock unavailable",
            )
        })?
        .as_nanos();
    let invocation = format!(
        "{}:{}:{}:{}:{}",
        context.run_id(),
        context.evaluated_commit(),
        context.cancellation_id(),
        std::process::id(),
        nonce
    );
    let run_dir = format!("training-{}", &sha256(invocation.as_bytes())[..24]);
    let store = CheckpointStore::create(context.artifact_directory(), &run_dir).map_err(|_| {
        failure(
            FailureClass::Reported,
            "training artifact directory unavailable",
        )
    })?;
    let checkpoint = store
        .save(
            &model,
            &corpus,
            SgdState {
                learning_rate: config.learning_rate,
            },
            config.steps,
        )
        .map_err(|_| failure(FailureClass::Reported, "training checkpoint unavailable"))?;
    let evidence_path = store.path().join("training-evidence.json");
    let evidence = BoundEvidence {
        schema_version: 1,
        run_id: context.run_id().as_str(),
        baseline_commit: context.baseline_commit().as_str(),
        evaluated_commit: context.evaluated_commit().as_str(),
        fixture_contract_sha256: corpus.contract_sha256(),
        config,
        validation: validation.clone(),
    };
    let evidence_bytes = serde_json::to_vec(&evidence)
        .map_err(|_| failure(FailureClass::Reported, "training evidence unavailable"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(evidence_path)
        .map_err(|_| failure(FailureClass::Reported, "training evidence unavailable"))?;
    file.write_all(&evidence_bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| failure(FailureClass::Reported, "training evidence unavailable"))?;
    build_output(context, &validation, &run_dir, &checkpoint)
}

fn build_output(
    context: &EvaluationContext,
    validation: &crate::ValidationEvidence,
    run_dir: &str,
    checkpoint: &Path,
) -> Result<EvaluatorOutput, EvaluatorFailure> {
    let objective = validation.objective.val_bpb;
    let runtime = u32::try_from(validation.diagnostics.runtime_millis)
        .map_err(|_| failure(FailureClass::Validation, "runtime exceeds bound"))?;
    let train_tokens = u32::try_from(validation.diagnostics.training_tokens)
        .map_err(|_| failure(FailureClass::Validation, "training tokens exceed bound"))?;
    let val_tokens = u32::try_from(validation.diagnostics.validation_tokens)
        .map_err(|_| failure(FailureClass::Validation, "validation tokens exceed bound"))?;
    let metrics = [
        Measurement::hard_gate("tiny_fixture_verified", true, None),
        Measurement::numeric(
            "val_bpb",
            NumericMetricKind::Objective,
            MetricDirection::Minimize,
            objective,
        ),
        Measurement::numeric(
            "runtime_millis",
            NumericMetricKind::Diagnostic,
            MetricDirection::Minimize,
            f64::from(runtime),
        ),
        Measurement::numeric(
            "training_tokens",
            NumericMetricKind::Diagnostic,
            MetricDirection::Maximize,
            f64::from(train_tokens),
        ),
        Measurement::numeric(
            "validation_tokens",
            NumericMetricKind::Diagnostic,
            MetricDirection::Maximize,
            f64::from(val_tokens),
        ),
    ];
    let measurements = metrics
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| failure(FailureClass::Validation, "training measurement invalid"))?;
    let checkpoint_name = checkpoint
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| failure(FailureClass::Reported, "checkpoint path unavailable"))?;
    Ok(EvaluatorOutput {
        evaluator_id: TRAINING_EVALUATOR_ID.into(),
        run_id: context.run_id().to_string(),
        baseline_commit: context.baseline_commit().to_string(),
        evaluated_commit: context.evaluated_commit().to_string(),
        measurements,
        observations: Vec::new(),
        artifacts: vec![
            Artifact {
                name: "training_evidence".into(),
                relative_path: format!("{run_dir}/training-evidence.json"),
                media_type: "application/json".into(),
            },
            Artifact {
                name: "training_checkpoint".into(),
                relative_path: format!("{run_dir}/{checkpoint_name}"),
                media_type: "application/json".into(),
            },
        ],
        warnings: vec![Warning {
            code: "peak_memory_unavailable".into(),
            detail: "portable CPU peak-memory measurement unavailable".into(),
        }],
    })
}

fn map_train_error(error: &TrainFailure) -> EvaluatorFailure {
    let class = match error {
        TrainFailure::Timeout => FailureClass::Timeout,
        TrainFailure::Cancelled => FailureClass::Cancelled,
        TrainFailure::InvalidConfig(_) => FailureClass::Validation,
        TrainFailure::Backend(_) | TrainFailure::NonfiniteLoss => FailureClass::Reported,
    };
    failure(class, "bounded training did not complete")
}

fn map_measure_error(error: &MeasureFailure) -> EvaluatorFailure {
    let class = match error {
        MeasureFailure::EnvironmentChanged | MeasureFailure::MissingSamples => {
            FailureClass::Validation
        }
        MeasureFailure::NonfiniteObjective | MeasureFailure::Backend(_) => FailureClass::Reported,
    };
    failure(class, "validation objective unavailable")
}

fn failure(class: FailureClass, detail: &'static str) -> EvaluatorFailure {
    EvaluatorFailure {
        class,
        detail: detail.into(),
    }
}
