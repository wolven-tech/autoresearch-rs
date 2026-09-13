//! Compact GPT shape, causality, finite loss, and deterministic CPU evidence.

use autoresearch_training_candle::{
    DeviceRequest, TinyCorpus, TinyGpt, TinyModelConfig, resolve_device,
};
use candle_core::{DType, Tensor};
use std::collections::BTreeSet;

#[test]
fn tiny_gpt_shape_parameter_count_and_deterministic_loss() {
    let corpus = TinyCorpus::embedded().expect("fixture");
    let device = resolve_device(DeviceRequest::Cpu).expect("CPU");
    let config = TinyModelConfig::for_corpus(&corpus);
    let first = TinyGpt::new(config, &device).expect("first model");
    let second = TinyGpt::new(config, &device).expect("second model");
    let batch = corpus.train_batch(&device).expect("batch");
    let logits = first.forward_logits(&batch.inputs).expect("logits");
    assert_eq!(logits.dims(), &[2, 8, 257]);
    assert_eq!(first.parameter_count(), 10_496);
    let metadata = serde_json::to_value(first.evidence()).expect("model evidence JSON");
    assert_eq!(metadata["config"]["attention_heads"], 2);
    assert_eq!(metadata["config"]["layers"], 1);
    assert_eq!(metadata["dtype"], "f32");
    assert_eq!(metadata["device"], "cpu");
    assert_eq!(metadata["parameter_count"], 10_496);
    assert_eq!(second.parameter_count(), first.parameter_count());
    let names = first
        .named_parameters()
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), first.named_parameters().len());
    let first_logits = logits
        .flatten_all()
        .expect("flatten")
        .to_vec1::<f32>()
        .expect("values");
    let second_logits = second
        .forward_logits(&batch.inputs)
        .expect("repeat logits")
        .flatten_all()
        .expect("flatten")
        .to_vec1::<f32>()
        .expect("repeat values");
    assert_eq!(first_logits, second_logits);
    let first_loss = first
        .loss(&batch.inputs, &batch.targets)
        .expect("loss")
        .to_scalar::<f32>()
        .expect("scalar");
    let second_loss = second
        .loss(&batch.inputs, &batch.targets)
        .expect("repeat loss")
        .to_scalar::<f32>()
        .expect("scalar");
    assert!(first_loss.is_finite());
    assert_eq!(first_loss.to_bits(), second_loss.to_bits());
}

#[test]
fn future_token_cannot_change_earlier_logits() {
    let corpus = TinyCorpus::embedded().expect("fixture");
    let device = resolve_device(DeviceRequest::Cpu).expect("CPU");
    let model = TinyGpt::new(TinyModelConfig::for_corpus(&corpus), &device).expect("model");
    let batch = corpus.train_batch(&device).expect("batch");
    let original = model
        .forward_logits(&batch.inputs)
        .expect("original logits");
    let mut rows = batch.inputs.to_vec2::<u32>().expect("input rows");
    rows[0][7] = 42;
    let changed = Tensor::from_vec(
        rows.into_iter().flatten().collect::<Vec<_>>(),
        (2, 8),
        &device,
    )
    .expect("changed input");
    let perturbed = model.forward_logits(&changed).expect("changed logits");
    let prefix = original
        .narrow(0, 0, 1)
        .expect("row")
        .narrow(1, 0, 7)
        .expect("prefix")
        .flatten_all()
        .expect("flatten")
        .to_vec1::<f32>()
        .expect("prefix values");
    let other_prefix = perturbed
        .narrow(0, 0, 1)
        .expect("row")
        .narrow(1, 0, 7)
        .expect("prefix")
        .flatten_all()
        .expect("flatten")
        .to_vec1::<f32>()
        .expect("prefix values");
    assert!(
        prefix
            .iter()
            .zip(other_prefix)
            .all(|(left, right)| (left - right).abs() < 1.0e-6)
    );
    let wrong_dtype = Tensor::zeros((1, 8), DType::F32, &device).expect("wrong dtype");
    assert!(model.forward_logits(&wrong_dtype).is_err());
}
