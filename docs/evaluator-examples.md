# Evaluator SDK examples

Run from repository root. Both examples use local fixtures only; no network,
deployment, market receipt, or production write is involved.

```text
cargo run -p autoresearch-evaluator --example native_evaluator
cargo build -p autoresearch-evaluator --example jsonl_evaluator
```

Native example: [`native_evaluator.rs`](../crates/autoresearch-evaluator/examples/native_evaluator.rs)
links a `NativeEvaluator` into caller, constructs a disposable context, and
passes result through `evaluate_native`. This is structural validation; a
runner must still match measurements against frozen manifest before creating
an `EvaluationSnapshot`.

JSONL example: [`jsonl_evaluator.rs`](../crates/autoresearch-evaluator/examples/jsonl_evaluator.rs)
reads one protocol-v1 request from stdin and writes one response to stdout.
Its `handle` function is tested against versioned golden request and response
records in [`tests/example.rs`](../crates/autoresearch-evaluator/tests/example.rs).
For an actual subprocess invocation, declare exact executable path, arguments,
timeout, hard gate, and objective metric in frozen manifest, then call
`evaluate_subprocess` with a validated `EvaluationContext` and bounded
`ProcessLimits`. Do not parse free-form stdout as a score. SDK never converts
an evaluator failure into a pass or numeric value.

SDK does not ship `autoresearch run` or `resume`. It also does not supply an
OS-level sandbox. Phase 4 adds product-web adapters; Phase 5 adds mutation
loop, journal/report integration, and end-to-end product experiments.
