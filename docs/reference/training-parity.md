# Tiny training parity boundary

Run opt-in cross-language comparison with:

```text
RUSTC_WRAPPER= cargo test -p autoresearch-training-candle --offline --test parity -- --ignored --nocapture
```

Python is required here as independent reference implementation. Script uses
standard library only; no Python package or network install. Rust/Candle and
Python consume same SHA-pinned fixture and seed 23. [Frozen report](../../fixtures/tiny/parity-report.json)
records device, dtype, numeric tolerances, observed differences, and known
semantic gaps.

Exact matches: train/validation token IDs, deterministic batch order, batch
inputs/targets. Tiny CPU forward logits and cross-entropy loss agree within
checked-in tolerances. One `output[0,0]` SGD coordinate agrees with Python
finite-difference gradient within tolerance. Python reference does not
implement full-parameter backpropagation or optimizer state, so full optimizer
parity is **not** demonstrated. After one optimizer step, Rust full-SGD loss
differs from Python selected-coordinate-only loss by 0.011730936 on checked-in
fixture; this is a known scope discrepancy, not a numeric tolerance pass.
Its f64 arithmetic after f32 initialization
also differs from Candle f32 math.

This test-specific model is not pinned upstream nanochat training: upstream
uses BPE, packing, RoPE, CUDA/FA3, and AdamW; this fixture uses UTF-8 byte
IDs, repeated fixed batch, learned positions, ordinary attention, and
stateless SGD. No H100 speed or full training parity claim follows from this
tiny fixture. Benchmark or production model claims require separate evidence.
