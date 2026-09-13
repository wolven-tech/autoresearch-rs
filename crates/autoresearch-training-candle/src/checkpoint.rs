//! Run-owned, bounded, checksum-verified tiny-model checkpoints.

use crate::{TinyCorpus, TinyGpt, TinyModelConfig, TrainingError, sha256};
use candle_core::{Device, Tensor};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

const MARKER: &str = ".autoresearch-run-owned";
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Stateless SGD optimizer state for exact resume.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SgdState {
    /// Fixed learning rate; no momentum buffers.
    pub learning_rate: f64,
}

impl SgdState {
    fn validate(self) -> Result<(), CheckpointError> {
        if !self.learning_rate.is_finite() || self.learning_rate <= 0.0 || self.learning_rate > 0.1
        {
            return Err(CheckpointError::Mismatch("invalid SGD learning rate"));
        }
        Ok(())
    }
}

/// Checkpoint failure; caller files are never replaced.
#[derive(Debug, Error)]
pub enum CheckpointError {
    /// Path or marker violates run ownership.
    #[error("unsafe checkpoint path: {0}")]
    UnsafePath(&'static str),
    /// Schema, hash, fixture, model, or optimizer mismatch.
    #[error("checkpoint mismatch: {0}")]
    Mismatch(&'static str),
    /// Encoded checkpoint exceeds fixed byte cap.
    #[error("checkpoint exceeds byte bound")]
    TooLarge,
    /// Invalid JSON.
    #[error("invalid checkpoint JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// File error.
    #[error("checkpoint I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Tensor error.
    #[error("checkpoint model: {0}")]
    Model(#[from] TrainingError),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameter {
    name: String,
    shape: Vec<usize>,
    values: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema_version: u8,
    model_config: TinyModelConfig,
    parameter_count: usize,
    device: String,
    dtype: String,
    optimizer: String,
    optimizer_state: SgdState,
    corpus_contract_sha256: String,
    completed_steps: usize,
    seed: u64,
    parameters: Vec<Parameter>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload_sha256: String,
    payload: Payload,
}

/// Exclusive directory for one run below existing artifact root.
#[derive(Debug)]
pub struct CheckpointStore {
    path: PathBuf,
    run_id: String,
}

impl CheckpointStore {
    /// Creates new run-owned directory, refusing an existing directory.
    ///
    /// # Errors
    ///
    /// Refuses symlink root, invalid ID, existing directory, or I/O error.
    pub fn create(root: &Path, run_id: &str) -> Result<Self, CheckpointError> {
        let root = safe_root(root)?;
        validate_id(run_id)?;
        let path = root.join(run_id);
        fs::create_dir(&path)?;
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(MARKER))?;
        marker.write_all(run_id.as_bytes())?;
        marker.sync_all()?;
        Ok(Self {
            path,
            run_id: run_id.into(),
        })
    }

    /// Reopens owned directory after validating marker and no symlink escape.
    ///
    /// # Errors
    ///
    /// Refuses unsafe or absent directory/marker.
    pub fn open(root: &Path, run_id: &str) -> Result<Self, CheckpointError> {
        let root = safe_root(root)?;
        validate_id(run_id)?;
        let path = root.join(run_id);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(CheckpointError::UnsafePath("run directory unsafe"));
        }
        let path = path.canonicalize()?;
        if !path.starts_with(&root) {
            return Err(CheckpointError::UnsafePath("run directory escapes root"));
        }
        let marker_path = path.join(MARKER);
        let marker_meta = fs::symlink_metadata(&marker_path)?;
        if !marker_meta.is_file() || marker_meta.file_type().is_symlink() || marker_meta.len() > 64
        {
            return Err(CheckpointError::UnsafePath("run marker unsafe"));
        }
        if fs::read(marker_path)? != run_id.as_bytes() {
            return Err(CheckpointError::UnsafePath("run marker mismatch"));
        }
        Ok(Self {
            path,
            run_id: run_id.into(),
        })
    }

    /// Canonical run-owned path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes one immutable checkpoint. Existing files are never replaced.
    ///
    /// # Errors
    ///
    /// Refuses ownership drift, invalid state, oversized content, or I/O.
    pub fn save(
        &self,
        model: &TinyGpt,
        corpus: &TinyCorpus,
        optimizer_state: SgdState,
        completed_steps: usize,
    ) -> Result<PathBuf, CheckpointError> {
        self.verify_owned()?;
        optimizer_state.validate()?;
        if completed_steps > 64 {
            return Err(CheckpointError::Mismatch("step exceeds tiny budget"));
        }
        let parameters = model
            .named_parameters()
            .iter()
            .map(|(name, var)| {
                let tensor = var.as_tensor();
                Ok(Parameter {
                    name: name.clone(),
                    shape: tensor.dims().to_vec(),
                    values: tensor.flatten_all()?.to_vec1::<f32>()?,
                })
            })
            .collect::<Result<Vec<_>, candle_core::Error>>()
            .map_err(TrainingError::from)?;
        let payload = Payload {
            schema_version: 1,
            model_config: model.config(),
            parameter_count: model.parameter_count(),
            device: "cpu".into(),
            dtype: "f32".into(),
            optimizer: "sgd_no_momentum".into(),
            optimizer_state,
            corpus_contract_sha256: corpus.contract_sha256().into(),
            completed_steps,
            seed: model.config().seed,
            parameters,
        };
        let payload_sha256 = sha256(&serde_json::to_vec(&payload)?);
        let bytes = serde_json::to_vec(&Envelope {
            payload_sha256,
            payload,
        })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
            return Err(CheckpointError::TooLarge);
        }
        let path = self.checkpoint_path(completed_steps);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(path)
    }

    /// Loads verified checkpoint into matching model; validation precedes mutation.
    ///
    /// # Errors
    ///
    /// Refuses corrupt, future-version, mismatched, symlinked, or oversized input.
    pub fn load(
        &self,
        model: &TinyGpt,
        corpus: &TinyCorpus,
        expected_optimizer: SgdState,
        completed_steps: usize,
    ) -> Result<(), CheckpointError> {
        self.verify_owned()?;
        expected_optimizer.validate()?;
        let path = self.checkpoint_path(completed_steps);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(CheckpointError::UnsafePath("checkpoint file unsafe"));
        }
        if metadata.len() > MAX_BYTES {
            return Err(CheckpointError::TooLarge);
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
            return Err(CheckpointError::TooLarge);
        }
        let envelope: Envelope = serde_json::from_slice(&bytes)?;
        let payload = envelope.payload;
        if sha256(&serde_json::to_vec(&payload)?) != envelope.payload_sha256 {
            return Err(CheckpointError::Mismatch("payload digest"));
        }
        if payload.schema_version != 1 {
            return Err(CheckpointError::Mismatch("unsupported schema version"));
        }
        if payload.model_config != model.config()
            || payload.parameter_count != model.parameter_count()
            || payload.device != "cpu"
            || payload.dtype != "f32"
            || payload.optimizer != "sgd_no_momentum"
            || payload.optimizer_state != expected_optimizer
            || payload.corpus_contract_sha256 != corpus.contract_sha256()
            || payload.completed_steps != completed_steps
            || payload.seed != model.config().seed
        {
            return Err(CheckpointError::Mismatch("checkpoint provenance"));
        }
        let named = model.named_parameters();
        if payload.parameters.len() != named.len() {
            return Err(CheckpointError::Mismatch("parameter count"));
        }
        let mut tensors = Vec::with_capacity(named.len());
        for (record, (name, var)) in payload.parameters.iter().zip(named) {
            if record.name != *name
                || record.shape != var.as_tensor().dims()
                || record.values.len() != var.as_tensor().elem_count()
                || record.values.iter().any(|value| !value.is_finite())
            {
                return Err(CheckpointError::Mismatch("parameter shape or value"));
            }
            tensors.push(
                Tensor::from_vec(record.values.clone(), record.shape.clone(), &Device::Cpu)
                    .map_err(TrainingError::from)?,
            );
        }
        for ((_, var), tensor) in named.iter().zip(tensors.iter()) {
            var.set(tensor).map_err(TrainingError::from)?;
        }
        Ok(())
    }

    fn verify_owned(&self) -> Result<(), CheckpointError> {
        let root = self
            .path
            .parent()
            .ok_or(CheckpointError::UnsafePath("missing root"))?;
        if Self::open(root, &self.run_id)?.path != self.path {
            return Err(CheckpointError::UnsafePath("run path changed"));
        }
        Ok(())
    }

    fn checkpoint_path(&self, step: usize) -> PathBuf {
        self.path.join(format!("checkpoint-step-{step}.json"))
    }
}

fn safe_root(root: &Path) -> Result<PathBuf, CheckpointError> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CheckpointError::UnsafePath("artifact root unsafe"));
    }
    let canonical = root.canonicalize()?;
    if canonical.parent().is_none() {
        return Err(CheckpointError::UnsafePath("artifact root too broad"));
    }
    Ok(canonical)
}

fn validate_id(run_id: &str) -> Result<(), CheckpointError> {
    if run_id.is_empty()
        || run_id.len() > 64
        || !run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(CheckpointError::UnsafePath("invalid run ID"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TinyModelConfig, TrainConfig, train_fixed_budget};
    use std::sync::atomic::AtomicBool;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "autoresearch-checkpoint-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("new owned test root");
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove owned test root");
        }
    }

    fn setup_model() -> (TinyCorpus, TinyGpt) {
        let corpus = TinyCorpus::embedded().expect("fixture");
        let model =
            TinyGpt::new(TinyModelConfig::for_corpus(&corpus), &Device::Cpu).expect("model");
        (corpus, model)
    }

    #[test]
    fn checkpoint_roundtrip_reproduces_next_step_exactly() {
        let root = TestRoot::new();
        let store = CheckpointStore::create(&root.0, "run-1").expect("owned run");
        let (corpus, staged) = setup_model();
        let (_, continuous) = setup_model();
        let (_, restored) = setup_model();
        let cancel = AtomicBool::new(false);
        let one = TrainConfig {
            steps: 1,
            ..TrainConfig::default()
        };
        let two = TrainConfig {
            steps: 2,
            ..TrainConfig::default()
        };
        train_fixed_budget(&staged, &corpus, one, &cancel).expect("first step");
        let path = store
            .save(
                &staged,
                &corpus,
                SgdState {
                    learning_rate: 0.01,
                },
                1,
            )
            .expect("save step one");
        assert!(path.starts_with(store.path()));
        let reopened = CheckpointStore::open(&root.0, "run-1").expect("reopen run");
        reopened
            .load(
                &restored,
                &corpus,
                SgdState {
                    learning_rate: 0.01,
                },
                1,
            )
            .expect("restore step one");
        let batch = corpus.train_batch(&Device::Cpu).expect("train batch");
        let staged_logits = staged
            .forward_logits(&batch.inputs)
            .expect("staged logits")
            .flatten_all()
            .expect("flat")
            .to_vec1::<f32>()
            .expect("vector");
        let restored_logits = restored
            .forward_logits(&batch.inputs)
            .expect("restored logits")
            .flatten_all()
            .expect("flat")
            .to_vec1::<f32>()
            .expect("vector");
        assert_eq!(staged_logits, restored_logits);
        let full = train_fixed_budget(&continuous, &corpus, two, &cancel).expect("two steps");
        let resumed = train_fixed_budget(&restored, &corpus, one, &cancel).expect("next step");
        assert!((full.losses[2] - resumed.losses[1]).abs() <= 1.0e-6);
    }

    #[test]
    fn rejects_corrupt_future_and_mismatched_without_overwriting() {
        let root = TestRoot::new();
        let store = CheckpointStore::create(&root.0, "run-2").expect("owned run");
        let (corpus, model) = setup_model();
        let (_, target) = setup_model();
        let state = SgdState {
            learning_rate: 0.01,
        };
        let path = store.save(&model, &corpus, state, 0).expect("save");
        let original = fs::read(&path).expect("checkpoint bytes");
        assert!(matches!(
            store.save(&model, &corpus, state, 0),
            Err(CheckpointError::Io(_))
        ));
        assert_eq!(original, fs::read(&path).expect("still original"));
        assert!(matches!(
            store.load(
                &target,
                &corpus,
                SgdState {
                    learning_rate: 0.02
                },
                0
            ),
            Err(CheckpointError::Mismatch(_))
        ));
        let mut envelope: Envelope = serde_json::from_slice(&original).expect("decode");
        envelope.payload.schema_version = 2;
        envelope.payload_sha256 = sha256(&serde_json::to_vec(&envelope.payload).expect("payload"));
        fs::write(&path, serde_json::to_vec(&envelope).expect("encode"))
            .expect("test future schema");
        assert!(matches!(
            store.load(&target, &corpus, state, 0),
            Err(CheckpointError::Mismatch("unsupported schema version"))
        ));
        fs::write(&path, b"{").expect("test corrupt JSON");
        assert!(matches!(
            store.load(&target, &corpus, state, 0),
            Err(CheckpointError::Json(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_run_and_checkpoint_without_touching_target() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new();
        let elsewhere = TestRoot::new();
        symlink(&elsewhere.0, root.0.join("escaped")).expect("test directory link");
        assert!(matches!(
            CheckpointStore::open(&root.0, "escaped"),
            Err(CheckpointError::UnsafePath(_))
        ));
        let store = CheckpointStore::create(&root.0, "run-3").expect("owned run");
        let caller_file = elsewhere.0.join("caller.txt");
        fs::write(&caller_file, b"untouched").expect("caller fixture");
        symlink(&caller_file, store.checkpoint_path(0)).expect("test checkpoint link");
        let (corpus, model) = setup_model();
        let state = SgdState {
            learning_rate: 0.01,
        };
        assert!(store.save(&model, &corpus, state, 0).is_err());
        assert!(matches!(
            store.load(&model, &corpus, state, 0),
            Err(CheckpointError::UnsafePath(_))
        ));
        assert_eq!(fs::read(&caller_file).expect("caller file"), b"untouched");
    }
}
