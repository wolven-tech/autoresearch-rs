# Training substrate

CPU-first Candle crate consumes only [SHA-pinned tiny fixture](../../fixtures/tiny/README.md)
at this stage. It exposes UTF-8 byte + BOS tokenization, deterministic
document order, shifted train/validation tensors, and explicit CPU/f32 policy.
CUDA, Metal, f16, and bf16 return unavailable rather than silently falling
back. Default tests need no network, downloaded corpus, or GPU.

Compact GPT forward path uses 257-token input/output vocabulary, eight
positions, 16 hidden channels, two causal heads, one pre-norm block, and 32
feed-forward channels (10,496 trainable f32 scalars). Initialization uses
frozen seed and named variable order. `ModelEvidence` records full shape,
device, dtype, and parameter count for evaluator provenance. Causal masking,
finite next-token loss, and deterministic CPU logits have tests.

Upstream Python retains trained BPE, best-fit packing, CUDA/FA3, and separate
training math. Position embeddings and ordinary softmax here are deliberately
not upstream RoPE/FA3. This crate does not yet assert semantic parity.

`train_fixed_budget` performs stateless SGD on repeated frozen first batch.
Default: four updates, 30-second cap, learning rate 0.01. Config rejects zero
or more than 64 steps, more than 300 seconds, invalid learning rates, and more
than 131,072 tokens. It checks cancellation and deadline before and after each
update, returns exact step/token count and finite loss trace only after full
completion. This intentionally differs from upstream AdamW and document
sampling; later parity report must keep that distinction visible. Candle
allocation errors return `TrainFailure::Backend`; evaluator process boundary
classifies process death as `non_zero_exit` and timeout as `timeout`. Neither
path yields an objective value.
