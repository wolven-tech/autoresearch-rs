//! CPU-first tiny-corpus pipeline. Byte-level test contract is deliberately
//! narrower than upstream BPE and CUDA training semantics.

use candle_core::{DType, Device, Tensor};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

mod model;
mod train;
pub use model::{ModelEvidence, TinyGpt, TinyModelConfig};
pub use train::{TrainConfig, TrainEvidence, TrainFailure, train_fixed_budget};

const CONTRACT: &[u8] = include_bytes!("../../../fixtures/tiny/contract.json");
const TRAIN: &[u8] = include_bytes!("../../../fixtures/tiny/train.txt");
const VALIDATION: &[u8] = include_bytes!("../../../fixtures/tiny/validation.txt");
const LCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const MAX_CONTRACT_BYTES: usize = 4 * 1024;
const MAX_CORPUS_BYTES: usize = 64 * 1024;

/// Unsupported backends fail explicitly rather than silently falling back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRequest {
    /// Default deterministic CPU path.
    Cpu,
    /// Reserved for explicitly implemented CUDA path.
    Cuda,
    /// Reserved for explicitly implemented Metal path.
    Metal,
}

/// Precision policy for reproducible tiny CPU calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionRequest {
    /// Verified tiny CPU path.
    F32,
    /// Reserved until fixture semantics and hardware validation exist.
    F16,
    /// Reserved until fixture semantics and hardware validation exist.
    Bf16,
}

/// Resolves only verified device contract.
///
/// # Errors
///
/// CUDA and Metal are unavailable until separately validated.
pub fn resolve_device(request: DeviceRequest) -> Result<Device, TrainingError> {
    match request {
        DeviceRequest::Cpu => Ok(Device::Cpu),
        DeviceRequest::Cuda | DeviceRequest::Metal => Err(TrainingError::UnavailableConfiguration),
    }
}

/// Resolves only verified numeric precision.
///
/// # Errors
///
/// F16 and BF16 are unavailable until separately validated.
pub const fn resolve_precision(request: PrecisionRequest) -> Result<DType, TrainingError> {
    match request {
        PrecisionRequest::F32 => Ok(DType::F32),
        PrecisionRequest::F16 | PrecisionRequest::Bf16 => {
            Err(TrainingError::UnavailableConfiguration)
        }
    }
}

/// Frozen tiny fixture dimensions and digests.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TinyContract {
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

impl TinyContract {
    /// Vocabulary includes one BOS token after all byte IDs.
    #[must_use]
    pub const fn vocab_size(&self) -> u32 {
        self.vocab_size
    }

    /// Fixed training context length.
    #[must_use]
    pub const fn sequence_length(&self) -> usize {
        self.sequence_length
    }

    /// Number of rows in deterministic first batch.
    #[must_use]
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Fixed seed used by wrapping-LCG Fisher–Yates ordering.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// BOS token ID.
    #[must_use]
    pub const fn bos_token_id(&self) -> u32 {
        self.bos_token_id
    }
}

/// Parsed and hash-validated train/validation documents.
#[derive(Debug, Clone)]
pub struct TinyCorpus {
    contract: TinyContract,
    train_tokens: Vec<Vec<u32>>,
    validation_tokens: Vec<Vec<u32>>,
    order: Vec<usize>,
    contract_sha256: String,
}

/// One fixed-width, shifted token batch. Mask is all one; no padding.
#[derive(Debug)]
pub struct TokenBatch {
    /// Source document indices in deterministic order.
    pub document_indices: Vec<usize>,
    /// `[batch, sequence]` token IDs.
    pub inputs: Tensor,
    /// `[batch, sequence]` next-token IDs.
    pub targets: Tensor,
    /// `[batch, sequence]` valid-token flags.
    pub mask: Tensor,
}

/// Invalid fixture or unsupported device does not produce training evidence.
#[derive(Debug, Error)]
pub enum TrainingError {
    /// Frozen JSON contract could not be parsed.
    #[error("tiny fixture contract invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// Text split is not UTF-8.
    #[error("tiny corpus is not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    /// Corpus hash no longer matches frozen contract.
    #[error("tiny corpus SHA-256 drift: {0}")]
    HashDrift(&'static str),
    /// Fixture dimensions, bytes, or tokenizer version unsupported.
    #[error("invalid tiny fixture: {0}")]
    InvalidFixture(&'static str),
    /// No verified backend semantics for requested device.
    #[error("requested device/dtype is unavailable; CPU f32 only")]
    UnavailableConfiguration,
    /// Candle tensor construction failed.
    #[error(transparent)]
    Candle(#[from] candle_core::Error),
}

impl TinyCorpus {
    /// Loads checked-in tiny fixture without network or generated data.
    ///
    /// # Errors
    ///
    /// Refuses malformed contract, hash drift, or unfillable batches.
    pub fn embedded() -> Result<Self, TrainingError> {
        Self::from_bytes(CONTRACT, TRAIN, VALIDATION)
    }

    /// Parses bounded in-memory fixture bytes for deterministic tests.
    ///
    /// # Errors
    ///
    /// Refuses malformed contract, hash drift, or unfillable batches.
    pub fn from_bytes(
        contract: &[u8],
        train: &[u8],
        validation: &[u8],
    ) -> Result<Self, TrainingError> {
        if contract.len() > MAX_CONTRACT_BYTES
            || train.len() > MAX_CORPUS_BYTES
            || validation.len() > MAX_CORPUS_BYTES
        {
            return Err(TrainingError::InvalidFixture("fixture exceeds byte bound"));
        }
        let parsed: TinyContract = serde_json::from_slice(contract)?;
        if parsed.schema_version != 1
            || parsed.tokenizer != "utf8_bytes_bos_v1"
            || parsed.normalization != "none"
            || parsed.bos_token_id != 256
            || parsed.vocab_size != 257
            || parsed.sequence_length == 0
            || parsed.sequence_length > 128
            || parsed.batch_size == 0
            || parsed.batch_size > 16
        {
            return Err(TrainingError::InvalidFixture("unsupported contract"));
        }
        if sha256(train) != parsed.train_sha256 {
            return Err(TrainingError::HashDrift("train"));
        }
        if sha256(validation) != parsed.validation_sha256 {
            return Err(TrainingError::HashDrift("validation"));
        }
        let train_tokens = tokenize_documents(train, parsed.bos_token_id)?;
        let validation_tokens = tokenize_documents(validation, parsed.bos_token_id)?;
        if parsed.batch_size > train_tokens.len() || validation_tokens.is_empty() {
            return Err(TrainingError::InvalidFixture("insufficient documents"));
        }
        let window = parsed.sequence_length + 1;
        if train_tokens
            .iter()
            .chain(&validation_tokens)
            .any(|row| row.len() < window)
        {
            return Err(TrainingError::InvalidFixture(
                "document shorter than sequence",
            ));
        }
        let order = shuffled_indices(train_tokens.len(), parsed.seed);
        Ok(Self {
            contract: parsed,
            train_tokens,
            validation_tokens,
            order,
            contract_sha256: sha256(contract),
        })
    }

    /// Exact validated contract.
    #[must_use]
    pub const fn contract(&self) -> &TinyContract {
        &self.contract
    }

    /// SHA-256 of exact JSON contract bytes.
    #[must_use]
    pub fn contract_sha256(&self) -> &str {
        &self.contract_sha256
    }

    /// Full BOS-prefixed training document tokens.
    #[must_use]
    pub fn train_token_ids(&self) -> &[Vec<u32>] {
        &self.train_tokens
    }

    /// Full BOS-prefixed validation document tokens.
    #[must_use]
    pub fn validation_token_ids(&self) -> &[Vec<u32>] {
        &self.validation_tokens
    }

    /// Complete deterministic document order for first epoch.
    #[must_use]
    pub fn batch_order(&self) -> &[usize] {
        &self.order
    }

    /// Materializes first fixed-width train batch on verified device.
    ///
    /// # Errors
    ///
    /// Returns Candle tensor construction failure.
    pub fn train_batch(&self, device: &Device) -> Result<TokenBatch, TrainingError> {
        self.batch_from_indices(
            &self.order[..self.contract.batch_size],
            &self.train_tokens,
            device,
        )
    }

    /// Materializes first validation row on verified device.
    ///
    /// # Errors
    ///
    /// Returns Candle tensor construction failure.
    pub fn validation_batch(&self, device: &Device) -> Result<TokenBatch, TrainingError> {
        self.batch_from_indices(&[0], &self.validation_tokens, device)
    }

    fn batch_from_indices(
        &self,
        indices: &[usize],
        source: &[Vec<u32>],
        device: &Device,
    ) -> Result<TokenBatch, TrainingError> {
        let sequence = self.contract.sequence_length;
        let mut inputs = Vec::with_capacity(indices.len() * sequence);
        let mut targets = Vec::with_capacity(indices.len() * sequence);
        for index in indices {
            let row = &source[*index];
            inputs.extend_from_slice(&row[..sequence]);
            targets.extend_from_slice(&row[1..=sequence]);
        }
        let shape = (indices.len(), sequence);
        Ok(TokenBatch {
            document_indices: indices.to_vec(),
            inputs: Tensor::from_vec(inputs, shape, device)?,
            targets: Tensor::from_vec(targets, shape, device)?,
            mask: Tensor::from_vec(vec![1_u8; indices.len() * sequence], shape, device)?,
        })
    }
}

/// Encodes one UTF-8 document as BOS + raw byte IDs.
#[must_use]
pub fn encode_document(document: &str, bos_token_id: u32) -> Vec<u32> {
    std::iter::once(bos_token_id)
        .chain(document.as_bytes().iter().copied().map(u32::from))
        .collect()
}

fn tokenize_documents(bytes: &[u8], bos: u32) -> Result<Vec<Vec<u32>>, TrainingError> {
    if !bytes.ends_with(b"\n") || bytes.contains(&b'\r') {
        return Err(TrainingError::InvalidFixture("corpus must use LF lines"));
    }
    let text = std::str::from_utf8(bytes)?;
    let documents = text.lines().collect::<Vec<_>>();
    if documents.is_empty() || documents.iter().any(|document| document.is_empty()) {
        return Err(TrainingError::InvalidFixture("blank document"));
    }
    Ok(documents
        .iter()
        .map(|document| encode_document(document, bos))
        .collect())
}

fn shuffled_indices(count: usize, seed: u64) -> Vec<usize> {
    let mut order = (0..count).collect::<Vec<_>>();
    let mut state = seed;
    for index in (1..count).rev() {
        state = state.wrapping_mul(LCG_MULTIPLIER).wrapping_add(1);
        let divisor = u64::try_from(index + 1).expect("bounded fixture");
        let other = usize::try_from(state % divisor).expect("bounded fixture");
        order.swap(index, other);
    }
    order
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
