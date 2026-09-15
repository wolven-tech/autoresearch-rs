# Explanation: Architecture

autoresearch-rs is a Rust implementation of Andrej Karpathy's autoresearch. It adds frozen inputs and Git isolation.

## Layers

Autoresearch-rs operates in layers from user to storage.

```text
User or Agent
    to
CLI (autoresearch-rs binary)
    to
Core (autoresearch-core)
    including Run state (worktrees, journal, locks)
    including Decision logic (gate evaluation, objective comparison)
    including Evaluator protocol (JSONL serialization)
    to
Git (git command-line tool)
    including Worktrees (isolated checkouts)
    including Refs (autoresearch/<run-id>)
    including Commits (candidate changes)
    to
Your Product Repository
    including autoresearch.toml (frozen contract)
    including program.md (frozen brief)
    including mutable paths (where changes go)
    including .autoresearch/ (run state)
```

## Core concepts

### Run

A run is a complete experiment from baseline to finish. It contains one run ID, one base commit frozen at baseline, one objective metric and one or more hard gates, one or more candidates evaluated sequentially, one journal recording append-only decisions, and one local ref `refs/heads/autoresearch/<run-id>` that moves on keep.

The run directory `.autoresearch/runs/<run-id>/` persists until you delete it. You can have multiple runs active at once with different run IDs.

### Baseline

Baseline freezes the contract and evaluates the base commit. It records the frozen manifest (a copy of autoresearch.toml), the frozen brief (a copy of program.md), the frozen identity (SHA256 hash of contract and environment), and the baseline measurements (evaluator output).

Baseline is immutable. Once frozen, the contract cannot change because candidates are rejected if they edit the manifest.

### Candidate

A candidate is one proposed change to the product. It gets its own detached worktree, starts at the base commit, has files edited under `mutable_paths`, gets committed automatically, is measured by the evaluator, has the decision journaled (keep or discard), and has the result applied in Git (branch advances or commit is orphaned).

Candidate worktrees are removed after finalization. Discarded commits are left as orphaned objects.

### Journal

The journal is an append-only JSONL file. Each line is a `JournalEntry` recording `baseline_captured` (baseline measurements recorded), `candidate_finalized` (candidate decided as keep or discard), `recovery_action` (recovery from a crash applied), or `stopped` (run was stopped).

The journal is immutable and durable. Every command that changes state appends to it. If the CLI crashes, `resume --run-id` can re-apply the last recorded decision without re-evaluating.

### Decision

A candidate goes through a fixed decision process. First the evaluator runs and measures the candidate. Next hard gates are checked, and if any fails, the candidate is discarded immediately. Then the objective metric is compared: if better the candidate is kept, if worse it is discarded, if tied then complexity decides (fewer changed lines keeps it, more discards it, equal lines move to dependency delta, equal delta move to runtime). Finally the decision is journaled before Git is touched, making it idempotent.

## Worktrees

Git worktrees isolate each evaluation. Autoresearch-rs uses three types: baseline worktrees created by `baseline` and left locked for reading the frozen state; candidate worktrees created by `run`, one per candidate, removed after finalization; and verification worktrees created by `verify` and left locked for fresh evaluation.

Worktrees are detached (not on any branch) and live in `.autoresearch/worktrees/<run-id>/`. Each has its own build directory, eliminating build-lock contention.

Worktrees provide isolation, where each evaluation has its own Git index and working tree without conflicts. Multiple runs can have active worktrees without interference. Worktrees can be removed cleanly with `git worktree remove`. Since worktrees share the Git object database, creation is fast.

## Artifact directory

Evaluators write build output to `artifact_directory` (from the request JSON), which lives outside the worktree. This enables warm caches where baseline and candidates share a cache keyed by commit hash. It keeps worktrees clean by preventing gitignored files from being left behind. It supports reproducibility by collecting build output for later inspection.

The artifact directory is shared across baseline, all candidates, and verification. Each gets a subdirectory keyed by commit SHA. Verification gets its own `verify/` subdirectory to build cold.

## Frozen identity

The frozen identity is a SHA256 hash computed from the frozen manifest (autoresearch.toml as plain text), the frozen brief (program.md as plain text), and the environment fingerprint (digest of env vars and git config).

Two runs with identical manifests and environment produce the same frozen identity. This enables reproducible snapshots and auditing.

## Evaluator protocol

Evaluators are standalone binaries. Autoresearch-rs communicates with them via JSONL. The CLI writes one JSON object to the evaluator's stdin and then closes it. The evaluator reads the request, parses the JSON, extracts run ID, commits, and paths. The evaluator measures the candidate by running tests and computing metrics. The evaluator writes one JSON object to stdout. The CLI reads the response, parses the JSON, extracts measurements, and applies decision logic.

The evaluator is stateless. It receives all needed context in the request and produces all needed output in the response. There is no database, no shared state, and no lingering processes.

## Locks

Autoresearch-rs uses file-based locking to serialize commands. The lock is `.autoresearch/run.lock`. Every command acquires it while working and releases when done.

Lock holding time is short (milliseconds to seconds for metadata operations, longer for evaluator runs). If a command crashes and leaves the lock held, you can delete the lock file and retry.

## Exit codes

Exit code 0 means success. Exit code 2 indicates a usage error from malformed arguments. Exit code 3 indicates a config error from an invalid manifest. Exit code 4 indicates an environment error such as a dirty tree, missing files, or evaluator crash. Exit code 5 means evaluation failed because the evaluator returned invalid JSON or verify drifted.

Exit codes are part of the contract, so scripts and agents can rely on them.

## Command flow

A user or agent calls the autoresearch CLI. This invokes autoresearch-core for decision logic. Core directs Git worktree operations, spawns your evaluator as a JSONL binary, and receives measurements as JSON back to the CLI. The outcome is journaled in an append-only log, and the decision updates Git refs under `autoresearch/<run-id>`.

Example commands flow through this sequence. First, initialize with `autoresearch init` to create the manifest and brief. Next, capture the baseline with `autoresearch baseline` to freeze the contract and evaluate at HEAD. Then, propose changes with `autoresearch run --run-id run-123` and advance the loop. Verify results with `autoresearch verify --run-id run-123`. Finally, report the run with `autoresearch report --run-id run-123` to produce JSON and HTML output.

Commands are typically invoked in this order. Create the manifest and brief with `autoresearch init`. Freeze the baseline with `autoresearch baseline`. Propose a candidate with `autoresearch run --run-id run-123 --hypothesis "my change"`. Verify results with `autoresearch verify --run-id run-123`. Generate a report with `autoresearch report --run-id run-123`.

## Data flow in a run

During baseline, manifest.toml, program.md, and environment converge to freeze the frozen_identity. The evaluator runs and produces measurements, which are journaled as baseline_captured.

During run submission, candidate edits are committed and sent to the evaluator for measurement. Decision logic determines disposition as keep or discard. The decision is journaled as candidate_finalized and applies a Git update-ref to move refs/heads/autoresearch/<run-id>.

During verify, the kept commit is evaluated fresh with the evaluator. Measurements are compared to the selection to determine status as matched or drifted. The result is journaled as verification_completed.

During report, the journal and measurements combine to produce JSON and HTML output.

## Error handling

Autoresearch-rs fails closed, meaning an unexpected condition produces an error rather than silent success. When the evaluator outputs malformed JSON, the failure class is `protocol`. When the evaluator times out, the failure class is `timeout`. When the evaluator exits with non-zero status, the failure class is `non_zero_exit`. When a required field is missing, the failure class is `validation`.

Every failure is journaled, and `resume --run-id` can retry the evaluation.

## Reproducibility

Because the manifest, brief, and environment are frozen, two runs with identical inputs produce identical baselines. The frozen identity hash proves this. To reproduce an experiment, use the frozen manifest from the run directory, match the frozen environment (git config and git version), run `baseline` on the same base commit, and the baseline measurements should match the journal.

Reproducibility is useful for auditing experiments months later, reproducing results in a different environment, and sharing experiments with others by sending the frozen manifest and evaluator.

## Upstream relationship

Autoresearch-rs is a Rust port of Andrej Karpathy's [karpathy/autoresearch](https://github.com/karpathy/autoresearch). The Python implementation is preserved in `reference/python/` with the pinned upstream commit recorded in `reference/python/MANIFEST.md`.

Key differences from upstream include: frozen manifest and brief (not mutable by candidates), Git worktrees (not clones), local-only refs (not pushed), expanded evaluator protocol (JSONL, gates, complexity measurement), and explicit environment freezing for reproducibility.

## Limits and constraints

Autoresearch-rs does not prove WCAG AA conformance in accessibility scores, and it does not prove rankings or AI citations from SEO and GEO results. It does not establish customer demand from lab numbers, and it does not count as product-bet promotion evidence. The tool provides no OS-level network isolation or filesystem sandbox for evaluators or agents, so untrusted code must run inside a sandbox you supply. Nothing is pushed or deployed by the tool. It is not autonomous orchestration: each `run` call advances one candidate, and nothing schedules the next call for you. The training example is not model parity and does not match upstream nanochat, CUDA, or H100 implementations.

For the decision algorithm and decision order, see how-a-run-decides.md. For directory structure, see run-state.md in the reference section.
