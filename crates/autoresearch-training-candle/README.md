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
