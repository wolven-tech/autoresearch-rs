//! Opt-in cross-language comparison, not upstream nanochat equivalence.

use autoresearch_training_candle::{
    TinyCorpus, TinyGpt, TinyModelConfig, TokenBatch, TrainConfig, train_fixed_budget,
};
use candle_core::Device;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;

#[test]
#[ignore = "opt-in Python/Candle semantic comparison"]
fn python_stdlib_and_candle_tiny_contract() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/tiny/python_model_reference.py");
    let output = Command::new("python3")
        .arg(script)
        .output()
        .expect("Python reference available for opt-in test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let python: Value = serde_json::from_slice(&output.stdout).expect("Python reference JSON");
    let report: Value =
        serde_json::from_slice(include_bytes!("../../../fixtures/tiny/parity-report.json"))
            .expect("checked-in parity report");
    assert_eq!(report["claim"], "tiny_fixture_partial_parity_only");
    assert_eq!(report["seed"], python["seed"]);
    let corpus = TinyCorpus::embedded().expect("frozen corpus");
    assert_eq!(
        python["train_token_ids"],
        serde_json::to_value(corpus.train_token_ids()).expect("train IDs")
    );
    assert_eq!(
        python["validation_token_ids"],
        serde_json::to_value(corpus.validation_token_ids()).expect("validation IDs")
    );
    assert_eq!(
        python["batch_indices"],
        serde_json::to_value(&corpus.batch_order()[..corpus.contract().batch_size()])
            .expect("batch indices")
    );
    let model =
        TinyGpt::new(TinyModelConfig::for_corpus(&corpus), &Device::Cpu).expect("Candle model");
    let batch = corpus.train_batch(&Device::Cpu).expect("Candle batch");
    assert_eq!(
        python["batch_inputs"],
        serde_json::to_value(batch.inputs.to_vec2::<u32>().expect("inputs")).expect("input JSON")
    );
    assert_eq!(
        python["batch_targets"],
        serde_json::to_value(batch.targets.to_vec2::<u32>().expect("targets"))
            .expect("target JSON")
    );
    let max_logit_error = max_logit_abs_error(&model, &batch, &python);
    let rust_loss = model
        .loss(&batch.inputs, &batch.targets)
        .expect("Candle loss")
        .to_scalar::<f32>()
        .expect("scalar loss");
    let python_loss = python["loss"].as_f64().expect("Python loss");
    let loss_error = (f64::from(rust_loss) - python_loss).abs();
    let (before_error, step_error, rust_gradient, python_gradient, full_step_loss_gap) =
        selected_step_differences(&model, &corpus, &batch, &python);
    println!(
        "max_logit_abs_error={max_logit_error:.9}; loss_abs_error={loss_error:.9}; selected_step_abs_error={step_error:.9}; python_gradient={python_gradient:.9}; rust_gradient={rust_gradient:.9}; full_vs_selected_step_loss_gap={full_step_loss_gap:.9}"
    );
    let tolerance = &report["tolerances"];
    assert!(
        before_error
            < tolerance["initial_parameter_abs"]
                .as_f64()
                .expect("initial tolerance")
    );
    assert!(
        max_logit_error
            < tolerance["forward_logit_max_abs"]
                .as_f64()
                .expect("logit tolerance")
    );
    assert!(
        loss_error
            < tolerance["forward_loss_abs"]
                .as_f64()
                .expect("loss tolerance")
    );
    assert!(
        step_error
            < tolerance["selected_sgd_step_abs"]
                .as_f64()
                .expect("step tolerance")
    );
    assert!(
        (rust_gradient - python_gradient).abs()
            < tolerance["selected_gradient_abs"]
                .as_f64()
                .expect("gradient tolerance")
    );
    assert_eq!(python["full_optimizer_step"], "not_implemented");
    assert!(full_step_loss_gap.is_finite());
}

fn max_logit_abs_error(model: &TinyGpt, batch: &TokenBatch, python: &Value) -> f64 {
    let rust_logits = model
        .forward_logits(&batch.inputs)
        .expect("Candle logits")
        .to_vec3::<f32>()
        .expect("logit cube");
    let python_logits = python["logits"].as_array().expect("Python batch logits");
    let mut max_error = 0.0_f64;
    for (rust_row, python_row) in rust_logits.iter().zip(python_logits) {
        for (rust_token, python_token) in
            rust_row.iter().zip(python_row.as_array().expect("tokens"))
        {
            for (rust_value, python_value) in rust_token
                .iter()
                .zip(python_token.as_array().expect("vocabulary"))
            {
                let delta = (f64::from(*rust_value) - python_value.as_f64().expect("logit")).abs();
                max_error = max_error.max(delta);
            }
        }
    }
    max_error
}

fn selected_step_differences(
    model: &TinyGpt,
    corpus: &TinyCorpus,
    batch: &TokenBatch,
    python: &Value,
) -> (f64, f64, f64, f64, f64) {
    let selected = model
        .named_parameters()
        .iter()
        .find(|(name, _)| name == "output")
        .expect("output matrix");
    let before = selected
        .1
        .as_tensor()
        .flatten_all()
        .expect("flat output")
        .to_vec1::<f32>()
        .expect("output values")[0];
    train_fixed_budget(
        model,
        corpus,
        TrainConfig {
            steps: 1,
            ..TrainConfig::default()
        },
        &AtomicBool::new(false),
    )
    .expect("one Candle SGD step");
    let after = selected
        .1
        .as_tensor()
        .flatten_all()
        .expect("flat updated output")
        .to_vec1::<f32>()
        .expect("updated output values")[0];
    let python_before = python["selected_before"].as_f64().expect("Python before");
    let python_after = python["selected_after_sgd"].as_f64().expect("Python after");
    let python_gradient = python["selected_gradient"]
        .as_f64()
        .expect("Python gradient");
    let rust_gradient = f64::from(before - after) / 0.01;
    let step_error = (f64::from(after) - python_after).abs();
    let rust_after_full_step_loss = model
        .loss(&batch.inputs, &batch.targets)
        .expect("full SGD loss")
        .to_scalar::<f32>()
        .expect("scalar full SGD loss");
    let python_after_selected_loss = python["loss_after_selected_update"]
        .as_f64()
        .expect("selected-coordinate Python loss");
    let full_step_loss_gap =
        (f64::from(rust_after_full_step_loss) - python_after_selected_loss).abs();
    (
        (f64::from(before) - python_before).abs(),
        step_error,
        rust_gradient,
        python_gradient,
        full_step_loss_gap,
    )
}
