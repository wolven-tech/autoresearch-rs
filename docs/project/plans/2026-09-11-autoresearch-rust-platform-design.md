# Autoresearch Rust platform design

Date: 11 September 2026

Status: approved

Repository: `wolven-tech/autoresearch-rs`

Upstream: `karpathy/autoresearch@228791fb499afffb54b46200aca536f79142f117`

## Intent

Preserve Karpathy autoresearch's useful invariant—freeze evaluation, change a
small surface, measure one comparable candidate, keep improvement, discard
regression—while making the loop useful beyond one Python GPU trainer.

The fork becomes a Rust-native experiment platform with two equal use cases:

1. Repository product research: correctness, UI, accessibility, performance,
   SEO, GEO, conversion clarity, and evidence quality.
2. LLM training research: a Candle translation of the upstream nanochat
   experiment behind the same evaluator contract.

The platform never converts internal scores, synthetic events, traffic,
impressions, clicks, or praise into product-gate evidence.

## Upstream preservation

GitHub fork ancestry remains intact and public. Original MIT attribution and
history remain visible. Current Python files move to `reference/python/` when
the Rust workspace becomes primary; a manifest records the upstream commit.
Tiny-corpus parity fixtures compare the reference implementation with the
Candle translation before any equivalence claim.

Relevant upstream sources:

- `README.md`
- `program.md`
- `prepare.py`
- `train.py`

## Workspace architecture

```text
crates/
  autoresearch-core
  autoresearch-config
  autoresearch-git
  autoresearch-runner
  autoresearch-evaluator
  autoresearch-report
  autoresearch-training-candle
apps/
  autoresearch-cli
examples/
  product-web
  nanochat-candle
reference/python/
```

### Crate responsibilities

- `autoresearch-core`: experiment states, frozen identities, typed metrics,
  gate results, candidate decisions, and journal records.
- `autoresearch-config`: versioned `autoresearch.toml` schema, validation,
  defaults, migration, and canonical hashing.
- `autoresearch-git`: base-ref validation, isolated worktrees, protected-path
  enforcement, candidate commits, retained branches, and safe cleanup.
- `autoresearch-runner`: mutation requests, evaluator scheduling, budgets,
  cancellation, recovery, and decision policy.
- `autoresearch-evaluator`: native evaluator traits and strict JSON Lines
  subprocess protocol.
- `autoresearch-report`: machine JSON, standalone HTML evidence board, and
  redacted export bundles.
- `autoresearch-training-candle`: compact GPT, tokenizer/data interfaces,
  optimizer, fixed-budget training, evaluation, checkpoints, and hardware
  measurements using Candle.
- `autoresearch-cli`: stable user commands and exit codes.

No dynamic-library plugin ABI ships in v1. Native adapters link statically;
external tools use versioned process protocol.

## Repository contract

Target repositories own two human-controlled files:

- `autoresearch.toml`: mutable paths, protected paths, commands, hard gates,
  objective, tie-breakers, environment, budget, network policy, and authority.
- `program.md`: research objective, domain context, candidate guidance, and
  known constraints.

Both become immutable after baseline. Runner hashes manifest, program,
fixtures, evaluator definitions, thresholds, metric direction, viewport set,
and original product gate. Any candidate modification invalidates itself.

Run state lives outside candidate worktrees under
`.autoresearch/runs/<run-id>/`. State includes baseline, append-only journal,
candidate manifests, command logs, typed results, screenshots, diffs,
environment fingerprint, decisions, and reports. Directory stays ignored by
default. Export creates a sanitised bundle.

## Mutation agents

Runner stays provider-neutral. Configured command adapters may invoke Codex,
Claude, another executable, or manual mutation. Runner passes only bounded
worktree path, allowed files, frozen rubric, previous decisions, and current
hypothesis. No vendor SDK or credentials enter core crates.

Mutation agents cannot change runner binary, evaluator, manifest, program,
baseline, lock policy, product gate, protected paths, or run journal.

## Evaluator protocol

JSON Lines request contains candidate path, run ID, baseline commit, changed
paths, declared environment, artifact directory, and cancellation identifier.
Response contains typed gates, metrics, observations, artifacts, warnings, and
failure classification. Malformed JSON, unexpected stdout, missing required
metric, timeout, or non-zero exit becomes evaluator failure.

Built-in adapters:

- Cargo formatting, Clippy, tests, builds, WASM boundaries, and dependency
  policy.
- Generic commands for TypeScript, Rust, browser, accessibility, security, and
  production checks.
- Web route, canonical, schema, robots, sitemap, focus, contrast, overflow,
  console, and screenshot checks.
- Lighthouse performance, accessibility, best-practices, SEO, and Web Vitals.
- GEO answer quality, entity consistency, citation-ready passages, source
  coverage, and self-report boundaries.
- Diff size, changed paths, dependency count, and complexity limits.
- Read-only market evidence imports for payment, fulfilment, refund, qualified
  use, and outreach receipts.
- Candle training validation bits-per-byte, runtime, tokens, memory, model
  shape, checkpoints, crash, and OOM state.

Metric kinds remain distinct:

- `hard_gate`: must pass.
- `objective`: drives keep or discard.
- `tie_breaker`: resolves equal objective.
- `diagnostic`: recorded only.
- `market_evidence`: recorded separately and never optimized or fabricated.

## Candidate policy

Selection is lexicographic:

1. Every hard gate passes.
2. Frozen primary objective improves in advances candidate.
3. Secondary metrics cannot offset primary regression.
4. Equal objective prefers smaller diff, fewer dependencies, then lower
   runtime.
5. Traffic, clicks, impressions, praise, lab scores, and synthetic telemetry
   never move a product gate.

Performance checks support warm-up, repeated samples, median aggregation, and
environment fingerprinting. Browser screenshots remain review artifacts.
Human review may veto a candidate but cannot rescue failed hard gates.

## Execution and recovery

Runner never edits caller checkout. It validates explicit base ref, creates a
run branch, and gives each candidate an isolated worktree. Kept candidate
advances `autoresearch/<run-id>`. Discarded candidate worktree is removed only
after target validation. Failed candidate retains logs and diff snapshot.

Every state transition is journaled before its side effect. Resume replays
journal and reruns only incomplete evaluator work. Runner never guesses about
partial Git decisions: each recorded decision binds exact evaluated candidate
commit, and finalization must match it. File locks prevent concurrent ownership
of same target and run branch.

Runs are serial by default. This preserves comparable machine conditions and
prevents product checks or GPU training from exhausting host resources.
Evaluators may declare resource groups for future controlled parallelism.

Stop conditions include experiment budget, failure budget, wall-clock budget,
signal cancellation, no valid mutable path, repeated invalid candidate, or
explicit operator stop. No unbounded default loop exists.

## Authority and process safety

External actions default denied. Deployment, outreach, purchases, payment
operations, permission changes, production writes, and product-gate changes
require explicit per-run authorization.

Executable allowlists, scrubbed environment, bounded output, process timeout,
child cancellation, redacted logs, symlink rejecting path validation, and
network-off evaluator defaults enforce scope. Secrets are available only
through named evaluator permissions and never appear in reports.

Runner rejects repository root deletion targets, home-directory targets,
unresolved variables, symlink escapes, nested repositories, dirty protected
files, and paths outside run-created worktrees.

## CLI

```text
autoresearch init
autoresearch doctor
autoresearch baseline
autoresearch run
autoresearch resume
autoresearch status
autoresearch report
autoresearch verify
autoresearch export
```

`init` creates commented config without overwriting. `doctor` validates tools,
Git, paths, evaluator availability, hardware, network policy, and protocol.
`baseline` freezes run inputs. `run` executes bounded loop. `resume` restores
from journal. `verify` independently reruns kept candidate. `report` rebuilds
evidence board. `export` creates redacted bundle.

## Evidence report

Standalone responsive HTML displays baseline, current best, candidate timeline,
hard-gate matrix, objective deltas, changed paths, diff size, runtime,
artifacts, screenshots, crash summaries, environment fingerprint, and explicit
decision reason. Market evidence occupies separate section from internal
capability. Report must never imply commercial validation from internal scores.

## Testing

- Unit: config validation, hashing, redaction, metric ordering, state machines.
- Property: path containment, journal replay, deterministic decisions.
- Integration: temporary Git repositories, crash recovery, worktree cleanup,
  allowed paths, command protocol, cancellation.
- Golden: JSON contracts and HTML reports.
- Browser: 320, 390, 768, and 1280 layouts; keyboard, reduced motion,
  overflow, and screenshot checks.
- Training: deterministic tiny corpus, fixed tokenizer fixture, loss-shape
  sanity, checkpoint round-trip, CPU smoke, optional CUDA comparison.
- End-to-end: product-web improvement and one nanochat parameter experiment.

## Delivery sequence

1. Foundation: workspace, config, journal, metric model, CLI.
2. Git loop: isolated worktrees, protected paths, branches, recovery.
3. Evaluator SDK: JSON protocol, command, Cargo, and diff adapters.
4. Product web: browser, Lighthouse, accessibility, SEO/GEO adapters and
   example.
5. Agent adapters: Codex, Claude, custom command, manual mode.
6. Report: static responsive evidence UI and redacted export.
7. Candle training: data, tokenizer, compact GPT, optimizer, fixed-budget run.
8. Parity: tiny-corpus Python/Rust comparison with documented discrepancies.
9. Portfolio pack: manifests for UI, copy, performance, SEO/GEO, calculator,
   and mobile research.

First usable release ends after report. Candle translation continues without
blocking product-research engine.

## Non-goals for first usable release

- Dynamic plugin loading.
- Distributed orchestration.
- Automatic production deployment or outreach.
- Automatic product promotion or gate changes.
- General-purpose workflow automation.
- Exact H100 performance parity before semantic parity fixtures pass.
