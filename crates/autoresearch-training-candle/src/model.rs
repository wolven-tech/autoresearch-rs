//! Deterministic f32 compact GPT for one frozen byte-level fixture.

use crate::{PrecisionRequest, TinyCorpus, TrainingError, resolve_precision};
use candle_core::{DType, Device, Tensor, Var};
use candle_nn::{Embedding, LayerNorm, Linear, Module, loss, ops};
use serde::{Deserialize, Serialize};

/// Explicit tiny transformer shape and deterministic initialization seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TinyModelConfig {
    /// Byte tokens plus BOS.
    pub vocab_size: usize,
    /// Maximum positions.
    pub sequence_length: usize,
    /// Hidden channels.
    pub embedding_dim: usize,
    /// Parallel causal attention heads.
    pub attention_heads: usize,
    /// Pre-norm transformer blocks.
    pub layers: usize,
    /// Feed-forward hidden channels.
    pub feed_forward_dim: usize,
    /// Fixed initializer state.
    pub seed: u64,
}

/// Exact model shape and precision for later evaluator provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelEvidence {
    /// Versioned model-metadata schema.
    pub schema_version: u8,
    /// Frozen or candidate model shape.
    pub config: TinyModelConfig,
    /// Only verified numeric precision.
    pub dtype: &'static str,
    /// Only verified backend.
    pub device: &'static str,
    /// Exact trainable scalar count.
    pub parameter_count: usize,
}

impl TinyModelConfig {
    /// Frozen small configuration for checked-in corpus.
    #[must_use]
    pub fn for_corpus(corpus: &TinyCorpus) -> Self {
        Self {
            vocab_size: 257,
            sequence_length: corpus.contract().sequence_length(),
            embedding_dim: 16,
            attention_heads: 2,
            layers: 1,
            feed_forward_dim: 32,
            seed: corpus.contract().seed(),
        }
    }

    fn validate(self) -> Result<Self, TrainingError> {
        if self.vocab_size != 257
            || self.sequence_length == 0
            || self.sequence_length > 128
            || self.embedding_dim == 0
            || self.embedding_dim > 64
            || self.attention_heads == 0
            || !self.embedding_dim.is_multiple_of(self.attention_heads)
            || self.layers == 0
            || self.layers > 4
            || self.feed_forward_dim == 0
            || self.feed_forward_dim > 256
        {
            return Err(TrainingError::InvalidFixture("unsupported model shape"));
        }
        Ok(self)
    }
}

struct TransformerBlock {
    attention_norm: LayerNorm,
    query: Linear,
    key: Linear,
    value: Linear,
    projection: Linear,
    feed_forward_norm: LayerNorm,
    feed_forward_in: Linear,
    feed_forward_out: Linear,
}

/// Trainable Candle model; all named parameters are owned, ordered, and f32.
pub struct TinyGpt {
    config: TinyModelConfig,
    token_embedding: Embedding,
    position_embedding: Tensor,
    blocks: Vec<TransformerBlock>,
    final_norm: LayerNorm,
    output: Linear,
    parameters: Vec<(String, Var)>,
    device: Device,
}

impl TinyGpt {
    /// Initializes fixed-size model deterministically on verified CPU f32.
    ///
    /// # Errors
    ///
    /// Refuses unsupported shape, device, dtype, or Candle tensor failure.
    pub fn new(config: TinyModelConfig, device: &Device) -> Result<Self, TrainingError> {
        let config = config.validate()?;
        if !device.is_cpu() || resolve_precision(PrecisionRequest::F32)? != DType::F32 {
            return Err(TrainingError::UnavailableConfiguration);
        }
        let mut init = Initializer::new(config.seed, device);
        let token_embedding = Embedding::new(
            init.weight("token_embedding", config.vocab_size, config.embedding_dim)?,
            config.embedding_dim,
        );
        let position_embedding = init.weight(
            "position_embedding",
            config.sequence_length,
            config.embedding_dim,
        )?;
        let mut blocks = Vec::with_capacity(config.layers);
        for index in 0..config.layers {
            let prefix = format!("blocks.{index}");
            let attention_norm =
                init.norm(&format!("{prefix}.attention_norm"), config.embedding_dim)?;
            let query = init.linear(
                &format!("{prefix}.query"),
                config.embedding_dim,
                config.embedding_dim,
            )?;
            let key = init.linear(
                &format!("{prefix}.key"),
                config.embedding_dim,
                config.embedding_dim,
            )?;
            let value = init.linear(
                &format!("{prefix}.value"),
                config.embedding_dim,
                config.embedding_dim,
            )?;
            let projection = init.linear(
                &format!("{prefix}.projection"),
                config.embedding_dim,
                config.embedding_dim,
            )?;
            let feed_forward_norm =
                init.norm(&format!("{prefix}.feed_forward_norm"), config.embedding_dim)?;
            let feed_forward_in = init.linear(
                &format!("{prefix}.feed_forward_in"),
                config.embedding_dim,
                config.feed_forward_dim,
            )?;
            let feed_forward_out = init.linear(
                &format!("{prefix}.feed_forward_out"),
                config.feed_forward_dim,
                config.embedding_dim,
            )?;
            blocks.push(TransformerBlock {
                attention_norm,
                query,
                key,
                value,
                projection,
                feed_forward_norm,
                feed_forward_in,
                feed_forward_out,
            });
        }
        let final_norm = init.norm("final_norm", config.embedding_dim)?;
        let output = init.linear("output", config.embedding_dim, config.vocab_size)?;
        Ok(Self {
            config,
            token_embedding,
            position_embedding,
            blocks,
            final_norm,
            output,
            parameters: init.parameters,
            device: device.clone(),
        })
    }

    /// Frozen shape metadata.
    #[must_use]
    pub const fn config(&self) -> TinyModelConfig {
        self.config
    }

    /// Exact trainable scalar count.
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        self.parameters
            .iter()
            .map(|(_, parameter)| parameter.as_tensor().elem_count())
            .sum()
    }

    /// Serializes exact shape, precision, and parameter count into evaluator evidence.
    #[must_use]
    pub fn evidence(&self) -> ModelEvidence {
        ModelEvidence {
            schema_version: 1,
            config: self.config,
            dtype: "f32",
            device: "cpu",
            parameter_count: self.parameter_count(),
        }
    }

    /// Ordered names and variables for bounded optimizer/checkpoint work.
    #[must_use]
    pub fn named_parameters(&self) -> &[(String, Var)] {
        &self.parameters
    }

    /// Computes `[batch, sequence, vocab]` logits with strict causal mask.
    ///
    /// # Errors
    ///
    /// Refuses wrong token tensor shape/dtype or Candle forward failure.
    pub fn forward_logits(&self, input_ids: &Tensor) -> Result<Tensor, TrainingError> {
        let [batch, sequence] = input_ids.dims() else {
            return Err(TrainingError::InvalidFixture(
                "input tensor must be rank two",
            ));
        };
        if *batch == 0
            || *sequence == 0
            || *sequence > self.config.sequence_length
            || input_ids.dtype() != DType::U32
            || !input_ids.device().same_device(&self.device)
        {
            return Err(TrainingError::InvalidFixture(
                "input tensor contract mismatch",
            ));
        }
        let token = self.token_embedding.forward(input_ids)?;
        let position = self
            .position_embedding
            .narrow(0, 0, *sequence)?
            .broadcast_left(*batch)?;
        let mut hidden = (token + position)?;
        for block in &self.blocks {
            hidden = block.forward(&hidden, self.config.attention_heads)?;
        }
        let logits = self.output.forward(&self.final_norm.forward(&hidden)?)?;
        Ok(logits)
    }

    /// Mean next-token cross entropy for fixed unpadded target tensor.
    ///
    /// # Errors
    ///
    /// Refuses target shape/dtype mismatch or nonfinite loss.
    pub fn loss(&self, input_ids: &Tensor, target_ids: &Tensor) -> Result<Tensor, TrainingError> {
        if input_ids.dims() != target_ids.dims() || target_ids.dtype() != DType::U32 {
            return Err(TrainingError::InvalidFixture(
                "target tensor contract mismatch",
            ));
        }
        let logits = self.forward_logits(input_ids)?;
        let tokens = input_ids.elem_count();
        let flattened = logits.reshape((tokens, self.config.vocab_size))?;
        let target = target_ids.flatten_all()?;
        let value = loss::cross_entropy(&flattened, &target)?;
        if !value.to_scalar::<f32>()?.is_finite() {
            return Err(TrainingError::InvalidFixture("nonfinite forward loss"));
        }
        Ok(value)
    }
}

impl TransformerBlock {
    fn forward(&self, input: &Tensor, heads: usize) -> Result<Tensor, TrainingError> {
        let [batch, sequence, channels] = input.dims() else {
            return Err(TrainingError::InvalidFixture(
                "hidden tensor shape mismatch",
            ));
        };
        let head_width = channels / heads;
        let normalized = self.attention_norm.forward(input)?;
        let query = self
            .query
            .forward(&normalized)?
            .reshape((*batch, *sequence, heads, head_width))?
            .transpose(1, 2)?
            .contiguous()?;
        let key = self
            .key
            .forward(&normalized)?
            .reshape((*batch, *sequence, heads, head_width))?
            .transpose(1, 2)?
            .contiguous()?;
        let value = self
            .value
            .forward(&normalized)?
            .reshape((*batch, *sequence, heads, head_width))?
            .transpose(1, 2)?
            .contiguous()?;
        let head_width_u32 = u32::try_from(head_width)
            .map_err(|_| TrainingError::InvalidFixture("head width too large"))?;
        let scores = query
            .matmul(&key.transpose(2, 3)?)?
            .affine(1.0 / f64::from(head_width_u32).sqrt(), 0.0)?;
        let mask = causal_mask(*sequence, input.device())?;
        let weights = ops::softmax_last_dim(&scores.broadcast_add(&mask)?)?;
        let attended = weights
            .matmul(&value)?
            .transpose(1, 2)?
            .reshape((*batch, *sequence, *channels))?;
        let projected = self.projection.forward(&attended)?;
        let residual = (input + projected)?;
        let feed_forward = self
            .feed_forward_in
            .forward(&self.feed_forward_norm.forward(&residual)?)?
            .relu()?
            .sqr()?;
        Ok((&residual + self.feed_forward_out.forward(&feed_forward)?)?)
    }
}

fn causal_mask(sequence: usize, device: &Device) -> Result<Tensor, TrainingError> {
    let values = (0..sequence)
        .flat_map(|row| {
            (0..sequence).map(move |column| if column <= row { 0.0_f32 } else { -1.0e9_f32 })
        })
        .collect::<Vec<_>>();
    Ok(Tensor::from_vec(
        values,
        (1, 1, sequence, sequence),
        device,
    )?)
}

struct Initializer<'a> {
    state: u64,
    device: &'a Device,
    parameters: Vec<(String, Var)>,
}

impl<'a> Initializer<'a> {
    fn new(seed: u64, device: &'a Device) -> Self {
        Self {
            state: seed,
            device,
            parameters: Vec::new(),
        }
    }

    fn weight(&mut self, name: &str, rows: usize, columns: usize) -> Result<Tensor, TrainingError> {
        let values = (0..rows * columns)
            .map(|_| self.next_weight())
            .collect::<Vec<_>>();
        self.parameter(name, values, (rows, columns))
    }

    fn linear(&mut self, name: &str, input: usize, output: usize) -> Result<Linear, TrainingError> {
        Ok(Linear::new(self.weight(name, output, input)?, None))
    }

    fn norm(&mut self, name: &str, channels: usize) -> Result<LayerNorm, TrainingError> {
        let gamma = self.parameter(&format!("{name}.gamma"), vec![1.0; channels], channels)?;
        let beta = self.parameter(&format!("{name}.beta"), vec![0.0; channels], channels)?;
        Ok(LayerNorm::new(gamma, beta, 1.0e-5))
    }

    fn parameter<S: Into<candle_core::Shape>>(
        &mut self,
        name: &str,
        values: Vec<f32>,
        shape: S,
    ) -> Result<Tensor, TrainingError> {
        let variable = Var::from_vec(values, shape, self.device)?;
        let tensor = variable.as_tensor().clone();
        self.parameters.push((name.into(), variable));
        Ok(tensor)
    }

    fn next_weight(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let raw = u16::try_from(self.state >> 48).expect("top bits fit u16");
        (f32::from(raw) / f32::from(u16::MAX) - 0.5) * 0.1
    }
}
