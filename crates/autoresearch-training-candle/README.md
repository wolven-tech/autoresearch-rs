# Training substrate

CPU-first Candle crate consumes only [SHA-pinned tiny fixture](../../fixtures/tiny/README.md)
at this stage. It exposes UTF-8 byte + BOS tokenization, deterministic
document order, shifted train/validation tensors, and explicit CPU/f32 policy.
CUDA, Metal, f16, and bf16 return unavailable rather than silently falling
back. Default tests need no network, downloaded corpus, or GPU.

Upstream Python retains trained BPE, best-fit packing, CUDA/FA3, and separate
training math. This crate does not yet assert semantic parity.
