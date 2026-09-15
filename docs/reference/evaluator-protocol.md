# Evaluator process protocol v1

External evaluators use one-request/one-response JSON Lines. Runner writes one
UTF-8 JSON object followed by `\n` to stdin, then closes stdin. Evaluator
writes exactly one UTF-8 JSON object followed by `\n` to stdout. Logs and
diagnostics go to stderr. Additional stdout lines, partial records, invalid
UTF-8, missing/unknown fields, and unsupported versions fail closed.

Golden records: [`request-v1.jsonl`](../../crates/autoresearch-evaluator/tests/fixtures/request-v1.jsonl),
[`response-success-v1.jsonl`](../../crates/autoresearch-evaluator/tests/fixtures/response-success-v1.jsonl),
and [`response-failure-v1.jsonl`](../../crates/autoresearch-evaluator/tests/fixtures/response-failure-v1.jsonl).

## Request fields

| Field | Meaning |
| --- | --- |
| `protocol_version` | Exact integer `1`. |
| `evaluator_id` | Frozen manifest evaluator identifier. |
| `run_id` | Stable experiment-run identifier. |
| `baseline_commit` | Full baseline Git SHA-1 or SHA-256 object ID. |
| `evaluated_commit` | Full Git object ID evaluated by this request. |
| `candidate_worktree` | Absolute isolated worktree path. |
| `changed_paths` | Repository-relative paths changed in evaluated commit. |
| `declared_environment` | Explicitly allowed key/value map; ambient process environment is not copied. |
| `artifact_directory` | Absolute run-owned output directory. |
| `cancellation_id` | Stable cancellation identifier for this invocation. |

## Response fields

Every response has `protocol_version: 1` and one `result` object. Successful
result uses `status: "success"` and `output`. Failed result uses
`status: "failure"` and `failure`; these shapes are mutually exclusive.

Successful `output` echoes `evaluator_id`, `run_id`, `baseline_commit`, and
`evaluated_commit`. It contains `measurements`, `observations`, `artifacts`,
and `warnings`. Each hard gate is encoded as
`{"kind":"hard_gate","name":"tests","outcome":{"passed":true,"detail":null}}`.
Each numeric result is encoded as
`{"kind":"numeric","name":"score","metric_kind":"objective","direction":"maximize","value":1.25}`.
Allowed numeric roles are `objective`, `tie_breaker`, and `diagnostic` for
evaluator output. `market_evidence` is reserved for separate read-only imports
and rejected by shared evaluator validation. Numeric values must be finite;
metric names, roles, and directions are checked against frozen manifest before
building a comparable snapshot.

`observations` and `warnings` carry `code` and `detail`. `artifacts` carry
`name`, `relative_path`, and `media_type`; artifact files must be inside
run-owned directory. `failure` carries a `class` and bounded diagnostic
`detail`. Classes include `reported`, `spawn`, `non_zero_exit`, `timeout`,
`cancelled`, `output_limit`, `protocol`, and `validation`. A reported failure
is not a failed hard gate and never produces an invented score.

## Compatibility

Version `1` accepts only documented fields and shapes. Unknown versions fail
closed. Any incompatible request or response change requires a new protocol
version, golden fixtures, explicit migration decision, and tests. Do not add
silent defaults to version `1`. Process execution limits and environment
scrubbing belong to SDK process runner, not to wire framing alone. SDK cannot
enforce OS-level network/filesystem isolation or kill arbitrary descendant
processes; operators must supply a trusted sandbox before running hostile code.

## Raw command adapter

Existing local checks that emit ordinary text can use exit-code gate mode:
exactly one declared hard gate, no numeric metrics. Exit zero records that
gate as passed. Non-zero exit is an evaluator failure, not a fabricated score
or a passed gate. Stdout and stderr remain bounded; stderr is redacted in
public diagnostics. Numeric outputs require JSONL protocol above.

## Cargo hard-gate adapter

`evaluate_cargo_check` supports four fixed checks: `cargo fmt --all --check`
(`cargo_format`), `cargo clippy --workspace --all-targets --all-features -- -D warnings`
(`cargo_clippy`), `cargo test --workspace --all-features` (`cargo_test`), and
`cargo build --workspace --all-features` (`cargo_build`). Each evaluator must
declare exactly its own gate, no numeric metrics, and exact literal arguments.
The adapter uses the same timeout, cancellation, bounded output, redaction,
and scrubbed environment as the raw command adapter. It runs from candidate
worktree, requires `CARGO_NET_OFFLINE=true`, and requires `CARGO_TARGET_DIR`
inside run-owned artifact directory. Unsupported flags, target, toolchain,
or absent offline configuration fail validation; nonzero Cargo exit fails
evaluation. This does not replace an OS-level process sandbox.

## Exact-commit diff evidence

`evaluate_commit_diff` reads only committed Git trees. Caller supplies exact
parent commit and frozen mutation boundary. Adapter requires candidate HEAD
to match evaluated commit, direct-child topology, clean worktree, and declared
changed paths equal derived diff. Rename evidence includes old and new paths;
binary paths stay visible but receive no invented line count. Text
`changed_lines` is additions plus deletions. Protected and out-of-scope paths
fail through same `MutationBoundary` rule used by Git candidate commits.

Dependency delta is known only for explicitly declared root-package
`Cargo.toml` direct dependency tables (`dependencies`, `dev-dependencies`,
`build-dependencies`). Workspace, target-specific, missing, malformed, or
other declared formats return `DependencyEvidence::Unavailable`. Caller
cannot create `Complexity` from unavailable evidence; it must choose an
explicit policy or report inability to compare. No dependency format is
guessed, and unknown delta never becomes zero.
