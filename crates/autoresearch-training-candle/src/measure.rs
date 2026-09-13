//! Frozen validation objective separated from runtime diagnostics.

use crate::{ModelEvidence, TinyCorpus, TinyGpt, TrainEvidence, TrainingError, sha256};
use candle_core::{Device, Tensor};
use serde::Serialize;
use std::time::Instant;
use thiserror::Error;

/// Only comparable objective from complete validation byte coverage.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationObjective {
    /// Mean negative log likelihood, converted from nats to bits per byte.
    pub val_bpb: f64,
    /// Exact number of target UTF-8 bytes represented in objective.
    pub evaluated_bytes: usize,
}

/// Non-objective diagnostics; absent memory means unavailable, never zero.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunDiagnostics {
    /// Wall-clock time since caller started training, rounded down to milliseconds.
    pub runtime_millis: u64,
    /// Exact completed training token budget.
    pub training_tokens: usize,
    /// Exact evaluated validation bytes.
    pub validation_tokens: usize,
    /// Unavailable on portable CPU path. No invented measurement.
    pub peak_memory_bytes: Option<u64>,
    /// Device, dtype, and full model shape.
    pub model: ModelEvidence,
    /// SHA-256 of explicit environment and corpus-contract fields.
    pub environment_fingerprint: String,
}

/// Complete successful result with objective isolated from diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationEvidence {
    /// Versioned evidence schema.
    pub schema_version: u8,
    /// Exact complete training trace used to produce model weights.
    pub training: TrainEvidence,
    /// Gate-comparable metric.
    pub objective: ValidationObjective,
    /// Non-gate operational measurements.
    pub diagnostics: RunDiagnostics,
}

/// Validation cannot return an objective when input or environment is invalid.
#[derive(Debug, Error)]
pub enum MeasureFailure {
    /// Exact environment declaration changed since run start.
    #[error("training environment fingerprint changed")]
    EnvironmentChanged,
    /// Missing or invalid validation samples.
    #[error("validation samples missing or invalid")]
    MissingSamples,
    /// Nonfinite metric cannot be used as objective.
    #[error("validation objective is nonfinite")]
    NonfiniteObjective,
    /// Backend or tensor failure.
    #[error("validation backend failure: {0}")]
    Backend(#[from] TrainingError),
}

#[derive(Serialize)]
struct FingerprintInputs<'a> {
    schema_version: u8,
    platform_os: &'static str,
    platform_arch: &'static str,
    package_version: &'static str,
    model: ModelEvidence,
    corpus_contract_sha256: &'a str,
}

/// Computes stable fingerprint from declared platform, model, and corpus.
///
/// # Errors
///
/// Serialization failure is treated as invalid fixture.
pub fn environment_fingerprint(
    model: &TinyGpt,
    corpus: &TinyCorpus,
) -> Result<String, TrainingError> {
    let inputs = FingerprintInputs {
        schema_version: 1,
        platform_os: std::env::consts::OS,
        platform_arch: std::env::consts::ARCH,
        package_version: env!("CARGO_PKG_VERSION"),
        model: model.evidence(),
        corpus_contract_sha256: corpus.contract_sha256(),
    };
    Ok(sha256(&serde_json::to_vec(&inputs)?))
}

/// Measures every frozen validation target byte after complete training.
///
/// Each document is split into non-overlapping blocks of at most model context
/// length. Position indices reset at block boundaries; no validation byte is
/// dropped. This is a tiny-fixture metric, not upstream validation semantics.
///
/// # Errors
///
/// Refuses changed fingerprint, empty/incomplete samples, nonfinite loss, or
/// backend failure. No partial objective is emitted.
pub fn measure_validation(
    model: &TinyGpt,
    corpus: &TinyCorpus,
    training: &TrainEvidence,
    started: Instant,
    expected_environment_fingerprint: &str,
) -> Result<ValidationEvidence, MeasureFailure> {
    let fingerprint = environment_fingerprint(model, corpus)?;
    if fingerprint != expected_environment_fingerprint {
        return Err(MeasureFailure::EnvironmentChanged);
    }
    if training.steps == 0 || training.losses.len() != training.steps + 1 {
        return Err(MeasureFailure::MissingSamples);
    }
    if training.losses.iter().any(|loss| !loss.is_finite()) {
        return Err(MeasureFailure::NonfiniteObjective);
    }
    let mut total_nats = 0.0_f64;
    let mut evaluated_bytes = 0_usize;
    let context = model.config().sequence_length;
    for document in corpus.validation_token_ids() {
        if document.len() < 2 {
            return Err(MeasureFailure::MissingSamples);
        }
        let target_count = document.len() - 1;
        for start in (0..target_count).step_by(context) {
            let end = (start + context).min(target_count);
            let count = end - start;
            let inputs = Tensor::from_vec(document[start..end].to_vec(), (1, count), &Device::Cpu)
                .map_err(TrainingError::from)?;
            let targets =
                Tensor::from_vec(document[start + 1..=end].to_vec(), (1, count), &Device::Cpu)
                    .map_err(TrainingError::from)?;
            let loss = model.loss(&inputs, &targets)?;
            let mean_nats = f64::from(loss.to_scalar::<f32>().map_err(TrainingError::from)?);
            if !mean_nats.is_finite() {
                return Err(MeasureFailure::NonfiniteObjective);
            }
            let count_u32 = u32::try_from(count).map_err(|_| MeasureFailure::MissingSamples)?;
            total_nats += mean_nats * f64::from(count_u32);
            evaluated_bytes += count;
        }
    }
    if evaluated_bytes == 0 {
        return Err(MeasureFailure::MissingSamples);
    }
    let evaluated_u32 =
        u32::try_from(evaluated_bytes).map_err(|_| MeasureFailure::MissingSamples)?;
    let val_bpb = total_nats / (f64::from(evaluated_u32) * std::f64::consts::LN_2);
    if !val_bpb.is_finite() {
        return Err(MeasureFailure::NonfiniteObjective);
    }
    let runtime_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(ValidationEvidence {
        schema_version: 1,
        training: training.clone(),
        objective: ValidationObjective {
            val_bpb,
            evaluated_bytes,
        },
        diagnostics: RunDiagnostics {
            runtime_millis,
            training_tokens: training.tokens_seen,
            validation_tokens: evaluated_bytes,
            peak_memory_bytes: None,
            model: model.evidence(),
            environment_fingerprint: fingerprint,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TinyModelConfig, TrainConfig, train_fixed_budget};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn complete_frozen_validation_is_finite_and_diagnostics_stay_separate() {
        let corpus = TinyCorpus::embedded().expect("fixture");
        let model =
            TinyGpt::new(TinyModelConfig::for_corpus(&corpus), &Device::Cpu).expect("model");
        let fingerprint = environment_fingerprint(&model, &corpus).expect("fingerprint");
        let started = Instant::now();
        let train = train_fixed_budget(
            &model,
            &corpus,
            TrainConfig::default(),
            &AtomicBool::new(false),
        )
        .expect("train");
        let result =
            measure_validation(&model, &corpus, &train, started, &fingerprint).expect("validate");
        let expected_bytes = corpus
            .validation_token_ids()
            .iter()
            .map(|row| row.len() - 1)
            .sum::<usize>();
        assert_eq!(result.objective.evaluated_bytes, expected_bytes);
        assert!(result.objective.val_bpb.is_finite());
        assert_eq!(result.diagnostics.training_tokens, train.tokens_seen);
        assert_eq!(result.diagnostics.validation_tokens, expected_bytes);
        assert_eq!(result.diagnostics.peak_memory_bytes, None);
        let json = serde_json::to_value(result).expect("serialize");
        assert!(json["objective"].get("val_bpb").is_some());
        assert!(json["objective"].get("runtime_millis").is_none());
        assert!(json["diagnostics"].get("runtime_millis").is_some());
        assert!(json["diagnostics"]["peak_memory_bytes"].is_null());
    }

    #[test]
    fn changed_fingerprint_and_missing_training_samples_fail_without_metric() {
        let corpus = TinyCorpus::embedded().expect("fixture");
        let model =
            TinyGpt::new(TinyModelConfig::for_corpus(&corpus), &Device::Cpu).expect("model");
        let train = TrainEvidence {
            schema_version: 1,
            steps: 1,
            tokens_seen: 16,
            losses: vec![1.0, 1.0],
            optimizer: "sgd_no_momentum",
            learning_rate: 0.01,
        };
        assert!(matches!(
            measure_validation(&model, &corpus, &train, Instant::now(), "changed"),
            Err(MeasureFailure::EnvironmentChanged)
        ));
        let fingerprint = environment_fingerprint(&model, &corpus).expect("fingerprint");
        assert!(matches!(
            measure_validation(
                &model,
                &corpus,
                &TrainEvidence {
                    losses: Vec::new(),
                    ..train.clone()
                },
                Instant::now(),
                &fingerprint
            ),
            Err(MeasureFailure::MissingSamples)
        ));
        assert!(matches!(
            measure_validation(
                &model,
                &corpus,
                &TrainEvidence {
                    losses: vec![f32::NAN, 1.0],
                    ..train
                },
                Instant::now(),
                &fingerprint
            ),
            Err(MeasureFailure::NonfiniteObjective)
        ));
    }
}
