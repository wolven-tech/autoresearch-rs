[PRD]
# PRD: Phase 3 Evaluator SDK

Status: proposed for implementation
Repository: `wolven-tech/autoresearch-rs`
Source of truth: `docs/project/plans/2026-09-11-autoresearch-rust-platform-design.md` and `.planning/ROADMAP.md`

## Overview

Phase 1 freezes comparable experiment contracts and typed decisions. Phase 2
isolates and recovers candidate Git worktrees. Phase 3 adds the missing
evaluation boundary: statically linked native evaluators and untrusted
subprocess evaluators return the same validated measurements. A generic command
adapter, a Cargo adapter, and a Git diff adapter make that boundary useful
without claiming that a candidate loop, browser evaluator, or product gate is
already operational.

## Goals

- Execute one declared evaluator against an exact, clean revision through a
  typed Rust API without editing the caller checkout.
- Define a versioned, documented one-request/one-response JSON Lines protocol
  for external evaluators.
- Reject malformed, incomplete, undeclared, or unsafe results before they can
  become an `EvaluationSnapshot` or a journal event.
- Classify process failures separately from legitimate failed hard gates.
- Supply command, Cargo, and diff adapters with deterministic, testable output.
- Preserve separation between internal evaluation and `MarketEvidence`.

## Scope decision

This phase delivers an SDK and adapters, not `autoresearch run` or `resume`.
The SDK exposes a runner-facing evaluation entry point; Phase 5 owns loop
orchestration, baseline/candidate journal writes, and CLI wiring. Phase 4 owns
browser, Lighthouse, accessibility, SEO, and GEO adapters. These boundaries
match the approved design and current roadmap.

## Quality Gates

### Epic-Level (run once on epic completion)

- `cargo fmt --all --check` passes.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- `cargo test --workspace --all-features` passes.
- Testing trophy below is complete; every required contract and subprocess
  failure case has a named, passing test.
- `git status --short` shows no unintended generated or run-state files.

### Story-Level (checked per story)

- Protocol/schema stories: fixture round-trip and rejection tests, including
  old/unknown protocol versions and extra stdout records.
- Subprocess stories: invoke controlled local fixture executables; assert exit
  classification, timeout, cancellation, output bounds, and environment policy.
- Adapter stories: execute against disposable repositories or fixture crates;
  assert exact typed measurements and changed-path provenance.
- Documentation story: compile and run the published minimal evaluator example
  against the current protocol fixture.

## User Stories

### US-001: Define evaluator context and native contract [Schema]

**Description:** As a runner author, I want one typed invocation/result shape
for native and subprocess evaluators so that no adapter can bypass validation.

**Acceptance Criteria:**

- [ ] Test first: context rejects blank run ID, invalid commit IDs, unsafe
  candidate/artifact paths, and duplicate changed paths
  (`crates/autoresearch-evaluator/tests/context.rs`).
- [ ] Add `crates/autoresearch-evaluator` to workspace with documented public
  `NativeEvaluator` trait and typed context, observation, artifact, warning,
  and failure types.
- [ ] Context carries run ID, baseline commit, exact evaluated commit,
  candidate worktree path, changed paths, declared environment, artifact
  directory, and cancellation identifier.
- [ ] Native and subprocess results enter one validator before snapshot
  construction; no dynamic-library ABI or vendor SDK is added.
- [ ] Context tests pass without invoking Git, network, or external services.

Mark each item [x] as you complete it. Only close when all are checked.

### US-002: Freeze JSONL wire protocol [Schema]

**Description:** As an evaluator author, I want a versioned process contract so
that an independent executable can return evidence without linking Rust code.

**Acceptance Criteria:**

- [ ] Test first: golden request/response fixtures round-trip and reject
  unsupported version, missing required field, unknown field, malformed JSON,
  multiple response records, and non-UTF-8 output
  (`crates/autoresearch-evaluator/tests/protocol.rs`).
- [ ] One newline-terminated JSON request is written to stdin; exactly one
  newline-terminated JSON response is accepted from stdout. Diagnostic text
  belongs on stderr, not stdout.
- [ ] Document protocol version `1`, field meanings, success and evaluator-
  reported failure shapes, numeric/gate encoding, and backward-compatibility
  rule in `docs/reference/evaluator-protocol.md`.
- [ ] Unknown protocol versions fail closed; version changes require a new
  fixture and explicit migration decision.
- [ ] Protocol tests pass with no shell parser or network dependency.

Mark each item [x] as you complete it. Only close when all are checked.

### US-003: Validate declared output and artifact provenance [Backend]

**Description:** As an experiment operator, I want only manifest-declared
measurements and run-owned artifacts accepted so that evaluator output cannot
move a frozen objective or smuggle files into a report.

**Acceptance Criteria:**

- [ ] Test first: reject missing/extra/duplicate metric and gate names, wrong
  kind or direction, NaN/infinity, baseline/candidate identity mismatch,
  absolute or escaping artifact path, symlink escape, and output outside run
  artifact directory (`crates/autoresearch-evaluator/tests/validation.rs`).
- [ ] Validate results against `ValidatedManifest::evaluators()` and existing
  `Measurement`, `MetricDefinition`, and `GateOutcome` types.
- [ ] Preserve manifest evaluator order and measurement order deterministically
  when constructing a complete `EvaluationSnapshot`.
- [ ] Keep `MarketEvidence` out of objective and hard-gate selection; imported
  commercial evidence cannot be emitted as an SDK-generated success signal.
- [ ] Validation tests pass for both native and subprocess result paths.

Mark each item [x] as you complete it. Only close when all are checked.

### US-004: Execute untrusted evaluator processes safely [Backend]

**Description:** As a runner author, I want bounded subprocess execution so
that a hung or noisy evaluator cannot exhaust host resources or leak secrets.

**Acceptance Criteria:**

- [ ] Test first: fixture executable covers success, non-zero exit, spawn
  failure, timeout, cancellation, malformed stdout, stderr-only diagnostics,
  and stdout/stderr limit (`crates/autoresearch-evaluator/tests/process.rs`).
- [ ] Use `CommandSpec` program and literal argument array; never interpolate
  shell text or execute through a shell by default.
- [ ] Start with scrubbed environment; pass only SDK-declared, explicitly
  allowed values. Do not copy ambient tokens, API keys, or credentials.
- [ ] Enforce configured non-zero timeout and bounded captured stdout/stderr;
  terminate owned process on timeout or cancellation and reap it.
- [ ] Classify process failure without converting it into a passed gate or
  fabricated numeric metric. Expose redacted diagnostics for later reporting.
- [ ] Process tests pass with no deployment, outreach, payment, or production
  write permissions.

Mark each item [x] as you complete it. Only close when all are checked.

### US-005: Add generic command adapter [Integration]

**Description:** As a product evaluator author, I want existing local checks
invoked through one adapter so that Rust and TypeScript commands can become
declared hard gates without bespoke runner code.

**Acceptance Criteria:**

- [ ] Test first: controlled command succeeds, fails, times out, emits extra
  stdout, and receives only allowed environment
  (`crates/autoresearch-evaluator/tests/command_adapter.rs`).
- [ ] Adapter executes in the candidate worktree only and binds its exact
  `CommandSpec` to a manifest evaluator ID.
- [ ] A zero exit can produce a declared pass gate; non-zero exit is reported
  as a failed evaluation or gate according to documented adapter policy and
  never yields an invented objective value.
- [ ] Numeric outputs require the versioned JSONL response; raw text parsing
  is not used to guess a score.
- [ ] Command adapter tests pass for a disposable Rust or TypeScript fixture.

Mark each item [x] as you complete it. Only close when all are checked.

### US-006: Add Cargo adapter [Integration]

**Description:** As a Rust repository maintainer, I want declared Cargo checks
to produce reproducible gate results without duplicating subprocess policy.

**Acceptance Criteria:**

- [ ] Test first: disposable Cargo fixture covers format, Clippy, test, and
  build pass/fail paths (`crates/autoresearch-evaluator/tests/cargo_adapter.rs`).
- [ ] Adapter constructs literal Cargo argument arrays for configured checks,
  runs in candidate worktree, and reuses subprocess timeout/output/env policy.
- [ ] Each check maps to an explicitly declared hard-gate name; one check
  cannot silently stand in for another.
- [ ] Unsupported target/toolchain configuration reports an unavailable or
  failed evaluator, not a pass.
- [ ] Cargo adapter tests pass without modifying source fixture or caller
  checkout.

Mark each item [x] as you complete it. Only close when all are checked.

### US-007: Add exact-commit diff adapter [Integration]

**Description:** As a decision engine, I want complexity inputs derived from
the evaluated commit so that equal objectives use real diff evidence.

**Acceptance Criteria:**

- [ ] Test first: disposable Git fixture covers additions, deletions,
  renames, binary files, unchanged files, and a candidate commit unrelated to
  the expected parent (`crates/autoresearch-evaluator/tests/diff_adapter.rs`).
- [ ] Diff binds parent and candidate commit IDs, uses committed tree content,
  and rejects dirty or ambiguous evaluator-visible state.
- [ ] Adapter produces deterministic changed paths and `changed_lines`
  (`additions + deletions`) for `Complexity`; binary-only changes are listed
  without invented line counts.
- [ ] `dependency_delta` is computed only from a supported, declared dependency
  manifest format; unsupported formats return explicit unavailable evidence
  rather than silently assuming zero.
- [ ] Diff adapter tests pass and preserve existing protected-path rules from
  `autoresearch-git`.

Mark each item [x] as you complete it. Only close when all are checked.

### US-008: Publish SDK example and phase handoff [Integration]

**Description:** As a contributor, I want a minimal working evaluator example
so that Phase 4 adapters and Phase 5 runner code can use the contract correctly.

**Acceptance Criteria:**

- [ ] Test first: documentation example compiles and exchanges protocol-v1
  fixture messages (`crates/autoresearch-evaluator/tests/example.rs`).
- [ ] Add a minimal statically linked native evaluator and a minimal
  subprocess evaluator example under `examples/` without external services.
- [ ] README distinguishes implemented SDK capability from unimplemented
  `run`, `resume`, browser, Lighthouse, and reporting features.
- [ ] `.planning/ROADMAP.md` marks Phase 3 complete only after all stories and
  epic-level gates pass; Phase 4 remains next.
- [ ] Example and documentation checks pass using only repository-local
  fixtures.

Mark each item [x] as you complete it. Only close when all are checked.

## Testing Trophy

### Unit — block merge

- [ ] Context, identifier, and artifact-path validation.
- [ ] Manifest output matching and metric-kind/direction checks.
- [ ] Failure classification and deterministic result ordering.
- [ ] Diff line accounting and supported dependency-manifest comparison.

### Contract — block merge

- [ ] JSONL v1 golden request/response round-trip.
- [ ] Unknown version/field, malformed, extra-record, and incomplete-output
  rejection.
- [ ] Native and subprocess result paths produce identical validated shapes.

### Integration — block merge

- [ ] Disposable process fixtures for timeout, cancellation, output caps,
  non-zero exits, environment scrubbing, and redacted diagnostics.
- [ ] Disposable Cargo fixture for format, Clippy, test, and build checks.
- [ ] Disposable Git fixture for exact-commit diff and containment checks.

### Frontend and E2E — not applicable in Phase 3

No browser UI or full mutation loop ships in this phase. Phase 4 adds browser
adapter tests; Phase 5 adds end-to-end run/journal/report tests. A protocol
example smoke check is required now, but it must not be presented as an
end-to-end product experiment.

## Functional Requirements

- FR-1: Every evaluator invocation binds frozen run ID, baseline commit,
  evaluated commit, candidate worktree, declared outputs, and artifact root.
- FR-2: Native and subprocess evaluators pass through the same output
  validator before any `EvaluationSnapshot` is returned.
- FR-3: Subprocess protocol accepts exactly one versioned JSONL response and
  rejects unexpected stdout, invalid UTF-8, wrong schema, or missing outputs.
- FR-4: Non-zero exit, timeout, cancellation, output overflow, and spawn
  failure are explicit failure classes, not comparable scores.
- FR-5: Subprocesses use literal arguments, scrubbed environment, bounded
  output, and run-owned working directory.
- FR-6: Command and Cargo adapters emit only declared hard gates and metrics.
- FR-7: Diff adapter derives complexity from exact committed trees and never
  treats unsupported dependency accounting as zero.
- FR-8: Market evidence remains separate from internal evaluator results and
  cannot move promotion or kill gates.

## Non-Goals

- `autoresearch run`/`resume` implementation, mutation agents, journal writes,
  candidate keep/discard orchestration, and final reporting.
- Browser, Lighthouse, accessibility, SEO, GEO, production, or market-receipt
  adapters.
- Dynamic plugins, remote evaluators, general secret injection, network-on
  defaults, deployment, payment, outreach, or automatic product-gate changes.
- Candle training and Python parity.

## Technical Considerations

- Reuse `ValidatedManifest`, `CommandSpec`, `Measurement`, `Complexity`,
  `EvaluationSnapshot`, and Phase 2 worktree/containment types. Do not duplicate
  Git safety policy or create a second metric schema.
- Manifest schema changes, if needed, must be optional and preserve existing
  v1 parsing and identity hashing. New output-affecting fields must change the
  frozen aggregate identity.
- Keep Rust 2024, workspace `unsafe_code = deny`, documented public APIs, and
  strict Clippy policy.
- Process-level network isolation is not guaranteed by `std::process::Command`.
  Until a trusted sandbox enforces it, Phase 3 must not claim network denial;
  keep evaluator network capabilities unavailable by default and surface this
  limitation in docs.

## Success Metrics

- All eight stories checked with named evidence and epic-level gates green.
- A third-party fixture executable can exchange protocol-v1 messages and has
  every malformed/failure class rejected deterministically.
- Cargo and diff adapters produce reproducible outputs from disposable
  fixtures; caller checkout remains unchanged.
- No CLI or README text claims baseline evaluator evidence before Phase 5
  connects SDK to journal and loop.

## Open Questions

- Should Phase 3 include thin CLI baseline execution? Default: no; retain
  approved roadmap boundary and deliver it with Phase 5 orchestration.
- Should dependency delta support non-Cargo manifests now? Default: only
  explicitly supported formats; unsupported formats fail visibly.
- Which host sandbox will enforce network-off and descendant-process cleanup?
  SDK must not claim stronger guarantees than verified platform behavior.
[/PRD]
