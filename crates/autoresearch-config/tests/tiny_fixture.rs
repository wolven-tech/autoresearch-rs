//! Frozen cross-language tiny corpus contract, without Python or GPU in default tests.

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const CONTRACT: &[u8] = include_bytes!("../../../fixtures/tiny/contract.json");
const TRAIN: &[u8] = include_bytes!("../../../fixtures/tiny/train.txt");
const VALIDATION: &[u8] = include_bytes!("../../../fixtures/tiny/validation.txt");
const GOLDEN: &str = include_str!("../../../fixtures/tiny/golden-v1.json");

#[derive(Deserialize)]
struct TinyContract {
    schema_version: u8,
    tokenizer: String,
    normalization: String,
    bos_token_id: u32,
    vocab_size: u32,
    seed: u64,
    sequence_length: usize,
    batch_size: usize,
    train_sha256: String,
    validation_sha256: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_bytes(contract: &TinyContract, train: &[u8], validation: &[u8]) -> Result<(), String> {
    if digest(train) != contract.train_sha256 {
        return Err("train corpus SHA-256 drift".into());
    }
    if digest(validation) != contract.validation_sha256 {
        return Err("validation corpus SHA-256 drift".into());
    }
    Ok(())
}

fn tokens(bytes: &[u8], bos: u32) -> Vec<Vec<u32>> {
    std::str::from_utf8(bytes)
        .expect("frozen UTF-8")
        .lines()
        .map(|document| {
            std::iter::once(bos)
                .chain(document.as_bytes().iter().copied().map(u32::from))
                .collect()
        })
        .collect()
}

fn shuffled_indices(count: usize, seed: u64) -> Vec<usize> {
    let mut order = (0..count).collect::<Vec<_>>();
    let mut state = seed;
    for index in (1..count).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let divisor = u64::try_from(index + 1).expect("small fixture");
        let other = usize::try_from(state % divisor).expect("small fixture");
        order.swap(index, other);
    }
    order
}

fn derive_fixture() -> Value {
    let contract: TinyContract = serde_json::from_slice(CONTRACT).expect("frozen contract");
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.tokenizer, "utf8_bytes_bos_v1");
    assert_eq!(contract.normalization, "none");
    assert_eq!(contract.bos_token_id, 256);
    assert_eq!(contract.vocab_size, 257);
    validate_bytes(&contract, TRAIN, VALIDATION).expect("frozen corpus digests");
    let train = tokens(TRAIN, contract.bos_token_id);
    let validation = tokens(VALIDATION, contract.bos_token_id);
    assert!(contract.batch_size > 0 && contract.batch_size <= train.len());
    let window = contract.sequence_length + 1;
    assert!(
        train
            .iter()
            .chain(&validation)
            .all(|row| row.len() >= window)
    );
    let order = shuffled_indices(train.len(), contract.seed);
    let batch_indices = &order[..contract.batch_size];
    let batch_inputs = batch_indices
        .iter()
        .map(|index| train[*index][..contract.sequence_length].to_vec())
        .collect::<Vec<_>>();
    let batch_targets = batch_indices
        .iter()
        .map(|index| train[*index][1..window].to_vec())
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "contract_sha256": digest(CONTRACT),
        "tokenizer": contract.tokenizer,
        "seed": contract.seed,
        "sequence_length": contract.sequence_length,
        "batch_size": contract.batch_size,
        "train_token_ids": train,
        "validation_token_ids": validation,
        "batch_indices": batch_indices,
        "batch_inputs": batch_inputs,
        "batch_targets": batch_targets,
        "batch_masks": vec![vec![1; contract.sequence_length]; contract.batch_size],
        "validation_inputs": &validation[0][..contract.sequence_length],
        "validation_targets": &validation[0][1..window],
    })
}

#[test]
fn frozen_python_golden_matches_two_deterministic_rust_replays() {
    let golden: Value = serde_json::from_str(GOLDEN).expect("Python golden JSON");
    assert_eq!(derive_fixture(), golden);
    assert_eq!(derive_fixture(), golden);
}

#[test]
fn frozen_corpus_hash_drift_is_rejected() {
    let contract: TinyContract = serde_json::from_slice(CONTRACT).expect("contract");
    let mut changed = TRAIN.to_vec();
    changed.push(b'!');
    assert_eq!(
        validate_bytes(&contract, &changed, VALIDATION),
        Err("train corpus SHA-256 drift".into())
    );
}
