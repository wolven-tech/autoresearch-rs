//! Frozen tiny data, batch, and device evidence on CPU only.

use autoresearch_training_candle::{
    DeviceRequest, PrecisionRequest, TinyCorpus, TrainingError, resolve_device, resolve_precision,
};
use serde_json::Value;

const GOLDEN: &str = include_str!("../../../fixtures/tiny/golden-v1.json");
const CONTRACT: &[u8] = include_bytes!("../../../fixtures/tiny/contract.json");
const TRAIN: &[u8] = include_bytes!("../../../fixtures/tiny/train.txt");
const VALIDATION: &[u8] = include_bytes!("../../../fixtures/tiny/validation.txt");

#[test]
fn candle_cpu_batches_match_python_golden_and_repeat() {
    let golden: Value = serde_json::from_str(GOLDEN).expect("Python golden");
    let device = resolve_device(DeviceRequest::Cpu).expect("CPU");
    for _ in 0..2 {
        let corpus = TinyCorpus::embedded().expect("frozen fixture");
        assert_eq!(corpus.contract_sha256(), golden["contract_sha256"]);
        assert_eq!(corpus.contract().vocab_size(), 257);
        assert_eq!(corpus.contract().sequence_length(), 8);
        assert_eq!(corpus.contract().batch_size(), 2);
        assert_eq!(corpus.contract().seed(), 23);
        assert_eq!(
            corpus.train_token_ids(),
            &golden["train_token_ids"]
                .as_array()
                .expect("tokens")
                .iter()
                .map(|row| {
                    row.as_array()
                        .expect("row")
                        .iter()
                        .map(|token| u32::try_from(token.as_u64().expect("token")).expect("u32"))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        );
        let batch = corpus.train_batch(&device).expect("train batch");
        assert_eq!(batch.document_indices, [2, 1]);
        assert_eq!(batch.inputs.dims(), &[2, 8]);
        assert_eq!(batch.targets.dims(), &[2, 8]);
        assert_eq!(batch.mask.dims(), &[2, 8]);
        assert_eq!(
            serde_json::to_value(batch.inputs.to_vec2::<u32>().expect("inputs")).expect("JSON"),
            golden["batch_inputs"]
        );
        assert_eq!(
            serde_json::to_value(batch.targets.to_vec2::<u32>().expect("targets")).expect("JSON"),
            golden["batch_targets"]
        );
        assert_eq!(
            serde_json::to_value(batch.mask.to_vec2::<u8>().expect("mask")).expect("JSON"),
            golden["batch_masks"]
        );
        let validation = corpus.validation_batch(&device).expect("validation batch");
        assert_eq!(validation.document_indices, [0]);
        assert_eq!(
            serde_json::to_value(
                validation
                    .inputs
                    .to_vec2::<u32>()
                    .expect("validation inputs")
            )
            .expect("JSON")[0],
            golden["validation_inputs"]
        );
        assert_eq!(
            serde_json::to_value(
                validation
                    .targets
                    .to_vec2::<u32>()
                    .expect("validation targets")
            )
            .expect("JSON")[0],
            golden["validation_targets"]
        );
    }
}

#[test]
fn drift_and_unsupported_devices_fail_closed() {
    let mut changed = TRAIN.to_vec();
    changed.push(b'!');
    assert!(matches!(
        TinyCorpus::from_bytes(CONTRACT, &changed, VALIDATION),
        Err(TrainingError::HashDrift("train"))
    ));
    assert!(matches!(
        resolve_device(DeviceRequest::Cuda),
        Err(TrainingError::UnavailableConfiguration)
    ));
    assert!(matches!(
        resolve_device(DeviceRequest::Metal),
        Err(TrainingError::UnavailableConfiguration)
    ));
    assert!(resolve_precision(PrecisionRequest::F32).is_ok());
    assert!(matches!(
        resolve_precision(PrecisionRequest::F16),
        Err(TrainingError::UnavailableConfiguration)
    ));
    assert!(matches!(
        resolve_precision(PrecisionRequest::Bf16),
        Err(TrainingError::UnavailableConfiguration)
    ));
}
