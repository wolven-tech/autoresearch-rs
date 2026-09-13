//! Bounded CPU training. Failures never carry an objective measurement.

use crate::{TinyCorpus, TinyGpt, TrainingError};
use candle_nn::{Optimizer, SGD};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Fixed-step SGD policy for the tiny fixture; no unbounded default.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainConfig {
    /// Number of optimizer updates, at most 64.
    pub steps: usize,
    /// Hard wall-clock cap, at most five minutes.
    pub max_wall_millis: u64,
    /// Stateless SGD learning rate in `(0, 0.1]`.
    pub learning_rate: f64,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            steps: 4,
            max_wall_millis: 30_000,
            learning_rate: 0.01,
        }
    }
}

/// Successful fixed-budget run. No failed run can construct this evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainEvidence {
    /// Frozen evidence schema.
    pub schema_version: u8,
    /// Exact completed optimizer updates.
    pub steps: usize,
    /// Tokens consumed, including repeats of frozen first batch.
    pub tokens_seen: usize,
    /// Loss before each update, followed by loss after final update.
    pub losses: Vec<f32>,
    /// Explicit optimizer, not upstream `AdamW`.
    pub optimizer: &'static str,
    /// Exact learning rate.
    pub learning_rate: f64,
}

/// Evaluation failure, never converted to a score or gate pass.
#[derive(Debug, Error)]
pub enum TrainFailure {
    /// Config cannot satisfy bounded contract.
    #[error("invalid bounded training configuration: {0}")]
    InvalidConfig(&'static str),
    /// Deadline passed before exact budget completed.
    #[error("training wall-clock deadline exceeded")]
    Timeout,
    /// Caller requested cancellation before exact budget completed.
    #[error("training cancelled")]
    Cancelled,
    /// Candle backend or fixture failure, including allocation failure.
    #[error("training backend failure: {0}")]
    Backend(#[from] TrainingError),
    /// Nonfinite measured loss cannot be an objective value.
    #[error("nonfinite training loss")]
    NonfiniteLoss,
}

/// Trains one already-initialized tiny model using repeated frozen first batch.
///
/// # Errors
///
/// Invalid bounds, timeout, cancellation, backend failure, and nonfinite loss
/// return errors without a partial `TrainEvidence`.
pub fn train_fixed_budget(
    model: &TinyGpt,
    corpus: &TinyCorpus,
    config: TrainConfig,
    cancelled: &AtomicBool,
) -> Result<TrainEvidence, TrainFailure> {
    validate(config, corpus)?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(config.max_wall_millis))
        .ok_or(TrainFailure::InvalidConfig("deadline overflow"))?;
    train_until(model, corpus, config, cancelled, deadline)
}

fn validate(config: TrainConfig, corpus: &TinyCorpus) -> Result<(), TrainFailure> {
    if !(1..=64).contains(&config.steps) {
        return Err(TrainFailure::InvalidConfig("steps must be 1..=64"));
    }
    if !(1..=300_000).contains(&config.max_wall_millis) {
        return Err(TrainFailure::InvalidConfig(
            "max_wall_millis must be 1..=300000",
        ));
    }
    if !config.learning_rate.is_finite()
        || !(0.0..=0.1).contains(&config.learning_rate)
        || config.learning_rate == 0.0
    {
        return Err(TrainFailure::InvalidConfig("invalid learning rate"));
    }
    let total = config
        .steps
        .checked_mul(corpus.contract().batch_size())
        .and_then(|value| value.checked_mul(corpus.contract().sequence_length()))
        .ok_or(TrainFailure::InvalidConfig("token budget overflow"))?;
    if total > 131_072 {
        return Err(TrainFailure::InvalidConfig("token budget exceeds cap"));
    }
    Ok(())
}

fn train_until(
    model: &TinyGpt,
    corpus: &TinyCorpus,
    config: TrainConfig,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<TrainEvidence, TrainFailure> {
    let device = candle_core::Device::Cpu;
    let batch = corpus.train_batch(&device)?;
    let vars = model
        .named_parameters()
        .iter()
        .map(|(_, variable)| variable.clone())
        .collect();
    let mut optimizer = SGD::new(vars, config.learning_rate).map_err(TrainingError::from)?;
    let mut losses = Vec::with_capacity(config.steps + 1);
    for _ in 0..config.steps {
        check_stop(cancelled, deadline)?;
        let loss = model.loss(&batch.inputs, &batch.targets)?;
        let value = loss.to_scalar::<f32>().map_err(TrainingError::from)?;
        if !value.is_finite() {
            return Err(TrainFailure::NonfiniteLoss);
        }
        optimizer
            .backward_step(&loss)
            .map_err(TrainingError::from)?;
        losses.push(value);
        check_stop(cancelled, deadline)?;
    }
    let final_loss = model.loss(&batch.inputs, &batch.targets)?;
    let value = final_loss.to_scalar::<f32>().map_err(TrainingError::from)?;
    if !value.is_finite() {
        return Err(TrainFailure::NonfiniteLoss);
    }
    check_stop(cancelled, deadline)?;
    losses.push(value);
    Ok(TrainEvidence {
        schema_version: 1,
        steps: config.steps,
        tokens_seen: config.steps * batch.inputs.elem_count(),
        losses,
        optimizer: "sgd_no_momentum",
        learning_rate: config.learning_rate,
    })
}

fn check_stop(cancelled: &AtomicBool, deadline: Instant) -> Result<(), TrainFailure> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TrainFailure::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(TrainFailure::Timeout);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TinyModelConfig;

    #[test]
    fn exact_budget_is_finite_and_reproducible() {
        let corpus = TinyCorpus::embedded().expect("embedded fixture");
        let config = TinyModelConfig::for_corpus(&corpus);
        let first = TinyGpt::new(config, &candle_core::Device::Cpu).expect("first model");
        let second = TinyGpt::new(config, &candle_core::Device::Cpu).expect("second model");
        let settings = TrainConfig::default();
        let cancel = AtomicBool::new(false);
        let first_run = train_fixed_budget(&first, &corpus, settings, &cancel).expect("first run");
        let second_run =
            train_fixed_budget(&second, &corpus, settings, &cancel).expect("second run");
        assert_eq!(first_run.steps, settings.steps);
        assert_eq!(first_run.tokens_seen, settings.steps * 16);
        assert_eq!(first_run.losses.len(), settings.steps + 1);
        assert!(first_run.losses.iter().all(|value| value.is_finite()));
        assert_eq!(first_run, second_run);
    }

    #[test]
    fn cancelled_timeout_and_invalid_budget_have_no_evidence() {
        let corpus = TinyCorpus::embedded().expect("embedded fixture");
        let model = TinyGpt::new(
            TinyModelConfig::for_corpus(&corpus),
            &candle_core::Device::Cpu,
        )
        .expect("model");
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            train_fixed_budget(&model, &corpus, TrainConfig::default(), &cancel),
            Err(TrainFailure::Cancelled)
        ));
        cancel.store(false, Ordering::Relaxed);
        assert!(matches!(
            train_until(
                &model,
                &corpus,
                TrainConfig::default(),
                &cancel,
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .expect("test deadline")
            ),
            Err(TrainFailure::Timeout)
        ));
        assert!(matches!(
            train_fixed_budget(
                &model,
                &corpus,
                TrainConfig {
                    steps: 0,
                    ..TrainConfig::default()
                },
                &cancel
            ),
            Err(TrainFailure::InvalidConfig(_))
        ));
    }
}
