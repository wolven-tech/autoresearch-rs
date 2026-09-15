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
not upstream RoPE/FA3. Tiny CPU token, batch, forward-logit, loss, and one
selected SGD-coordinate checks are documented; full optimizer parity is not.

`train_fixed_budget` performs stateless SGD on repeated frozen first batch.
Default: four updates, 30-second cap, learning rate 0.01. Config rejects zero
or more than 64 steps, more than 300 seconds, invalid learning rates, and more
than 131,072 tokens. It checks cancellation and deadline before and after each
update, returns exact step/token count and finite loss trace only after full
completion. This intentionally differs from upstream AdamW and document
sampling; the parity report keeps that distinction visible. Candle
allocation errors return `TrainFailure::Backend`; evaluator process boundary
classifies process death as `non_zero_exit` and timeout as `timeout`. Neither
path yields an objective value.

`measure_validation` evaluates every target byte in frozen validation text,
splitting documents into non-overlapping context blocks with reset position
indices. `val_bpb` is mean next-byte negative log likelihood in bits. It is
stored alone under `objective`; runtime, training/validation token counts,
model shape, CPU/f32, environment fingerprint, and unavailable (`null`) peak
memory sit under `diagnostics`. Exact training trace is attached. A changed
environment fingerprint, missing sample, or nonfinite loss returns an error
without metric. This byte-level objective must not be compared directly with
upstream BPE validation scores.

`CheckpointStore` creates one exclusive run-owned directory beneath an
existing artifact root. Immutable `checkpoint-step-N.json` files include
schema, shape, CPU/f32, ordered f32 parameters, stateless SGD learning rate,
fixture-contract SHA, step, and seed under a payload SHA. Save uses
`create_new`; load verifies size, marker, digest, provenance, parameter names,
shapes, and finite values before changing model state. Stable symlinks and
future schemas fail closed. Fixture test verifies resumed next-step logits
exactly and loss within 1e-6 of uninterrupted two-step run. This checkpoint
does not imply portable optimizer-state parity with upstream AdamW.

`TrainingEvaluator` reads only candidate-worktree `training-config.json` and
maps `val_bpb` to frozen objective, runtime and token counts to diagnostics,
and full evidence/checkpoint to run-owned artifacts. Evidence records exact
baseline/evaluated commit IDs and corpus-contract SHA. Peak memory remains
`null` in evidence, with explicit unavailable warning; no numeric zero is
emitted. `autoresearch-training-evaluator` wraps same adapter in Phase 3
bounded JSONL protocol for Phase 5 runner. Example config:

```json
{"steps":4,"max_wall_millis":30000,"learning_rate":0.01}
```

Declare evaluator ID `candle_tiny_training`, hard gate
`tiny_fixture_verified`, objective `val_bpb` minimized, tie-breaker
`parameter_count` minimized, and diagnostic metrics
`runtime_millis` minimized plus `training_tokens` and `validation_tokens`
maximized. Process mode matters for resource failures: crash or OS OOM kill
becomes a nonzero-exit failure; timeout and cancellation terminate child.
None yields a comparable objective or passed gate. Native mode is suitable
only for trusted in-process fixture tests, not resource isolation.

Cross-language status and exact limits: [tiny parity report](../../docs/reference/training-parity.md).
