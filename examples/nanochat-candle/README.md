# Disposable tiny Candle candidate

Copy these three files into a **new disposable Git repository**, not a product
repository. Set evaluator `program` to an absolute path of locally built
`autoresearch-training-evaluator`; CLI subprocess mode has its own scrubbed
environment and timeout. Freeze baseline, prepare candidate, change only
`training-config.json`, then submit hypothesis, verify retained commit, and
render report. The bounded fixture test covers this path. `val_bpb` is the
sole primary objective; parameter count is tie-breaker. Runtime and token
counts are diagnostics. `tiny_fixture_verified` is a local integrity gate.

This uses SHA-pinned embedded tiny text, CPU/f32, and one bounded candidate.
No customer, payment, market, or product bet promotion gate changes. Optional
CUDA comparison is deliberately **not** in CI: this crate returns
`UnavailableConfiguration` for CUDA/Metal/f16/bf16, and this host has no
validated CUDA training path. A future hardware comparison needs a separate
explicit fixture, precision policy, and measured device provenance; no H100
claim is available.
