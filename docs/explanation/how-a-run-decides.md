# Explanation: How a Run Decides

Every candidate goes through a fixed decision process. Understanding this process helps you write better evaluators and interpret results.

## The decision flowchart

```text
candidate evaluates
    ↓
hard gates pass?
  ↙ no → DISCARD (failed_hard_gates)
  ↘ yes
    ↓
objective vs current best
  ↙ better → KEEP (primary_improvement)
  ↓ worse → DISCARD (primary_regression)
  ↘ tie
    ↓
changed lines vs current best
  ↙ fewer → KEEP (tie_break_lines)
  ↓ more → DISCARD (tie_break_lines)
  ↘ equal
    ↓
runtime vs current best
  ↙ faster → KEEP (tie_break_runtime)
  ↓ slower → DISCARD (tie_break_runtime)
  ↘ equal → DISCARD (no_improvement)
```

## 1. Hard gates first

Hard gates are pass/fail checks that every candidate must pass. If any gate fails, the candidate is discarded immediately, and the objective is never compared.

A candidate that improves the objective but fails a gate is still discarded. This is by design: a gate is a requirement, not a suggestion.

Example from the README:

```text
| 1 | Critical CSS/font plus contained timeline and content-crossing shimmer | 15 | Reject — Axe marked all 15 rail text nodes contrast-indeterminate. |
```

The candidate scored 15 (down from 29) but was rejected because an accessibility gate failed.

## 2. Objective comparison

If all gates pass, the candidate's objective value is compared to the current best. The objective is the single metric declared in the manifest with `kind = "objective"` and `direction = "minimize"` or `"maximize"`.

The comparison is exact with no epsilon and no noise tolerance. An improvement in the declared direction results in a keep with reason `primary_improvement`. A regression results in discard with reason `primary_regression`. An exact tie proceeds to tie-breaking.

The measurement is a single number. If the evaluator measures 1.0 and the current best is 1.0, that is an exact tie, even if the underlying measurement (e.g., five Lighthouse samples) has noise. If you want to handle noise, you need to emit a stable statistic (a mean, median, or deterministic algorithm) instead of a raw sample.

## 3. Tie-breaking on changed lines

When the objective is exactly tied, the candidate's line count is compared to the current best's line count. Line count is the number of lines added or removed in the diff.

- **Fewer lines** → **keep** with reason `tie_break_lines`. The candidate achieved the same objective with less churn.
- **More lines** → **discard**. The current best achieved the same objective with less churn.
- **Equal lines** → proceed to runtime tie-breaking.

This rule prevents meaningless tie-wins. A candidate that shuffles whitespace and achieves the same objective scores zero improvement: it tied on the metric but changed lines, so it loses to the current best which achieved the same metric without changing.

### Example

Baseline: `paragraph_count = 2.0`, `changed_lines = 0`

Candidate 1: `paragraph_count = 1.0` (improvement) → keep
  - Decision: `primary_improvement`
  - Recorded: `changed_lines = 3`

Candidate 2: `paragraph_count = 1.0` (ties candidate 1) → compare lines
  - Candidate 2 changed 2 lines
  - Candidate 1 changed 3 lines
  - 2 < 3 → keep candidate 2
  - Decision: `tie_break_lines`

## 4. Tie-breaking on runtime

When objective and line count are tied, runtime decides. Runtime is wall-clock milliseconds from when the evaluator started until it exited. It does not include Git operations, complexity measurement, or decision overhead.

- **Faster** (fewer milliseconds) → **keep** with reason `tie_break_runtime`. The candidate achieved the same metric with the same complexity in less time (faster evaluator, not faster software).
- **Slower** → **discard**.
- **Equal runtime** → **discard** with reason `no_improvement`.

Runtime is rarely the real differentiator. It is useful as a last resort when two candidates are identical on the metric and complexity. In most real experiments, the line count tie-break decides the winner.

## Ties against baseline

The baseline has `changed_lines = 0`. This means no candidate can win a line-count tie against the baseline; ties against the baseline always reach the runtime step, and the baseline has `runtime = 0` (no evaluation time for a frozen snapshot), so the tie-break always discards the candidate.

This is correct: an identical objective with no line change to the baseline is not an improvement. If something worth keeping can't move the metric, it's not worth keeping.

## Ties against a kept candidate

Once a candidate is kept, the current best is no longer the baseline. A new candidate that ties the kept candidate goes to line-count tie-breaking against the kept candidate's line count, not the baseline's.

Timeline:

```text
Baseline: paragraph_count = 2.0, changed_lines = 0
Candidate 1: paragraph_count = 1.0, changed_lines = 3 → keep (primary_improvement)
Candidate 2: paragraph_count = 1.0, changed_lines = 2 → compare to candidate 1
  → candidate 2 changed fewer lines → keep (tie_break_lines)
Candidate 3: paragraph_count = 1.0, changed_lines = 2 → compare to candidate 2
  → same lines → compare runtime
  → if same runtime → discard (no_improvement)
```

## No epsilon, no noise tolerance

Because comparison is exact, small jitter shows up as a loss. If your evaluator samples a metric that varies by a few points between runs (e.g., Lighthouse score), the first sample will tie and enter line-count breaking, and `verify` will report drift when the fresh run samples differently.

This signals that your evaluator is not stable enough for exact comparison. To handle jitter, you have several options:

1. **Emit a stable statistic** — Instead of one Lighthouse sample, run five and report the mean (or median, or any deterministic algorithm). The mean is not random and will compare exactly across runs.

2. **Move the metric to tie_breaker or diagnostic role** — It is still recorded and visible in reports, but it does not drive the decision.

3. **Add a gate that enforces stability** — If the metric must be precise, add a gate that fails if the value drifts more than a tolerance between consecutive runs.

4. **Accept drift as a feature** — If your experiment is fundamentally noisy (testing a probabilistic algorithm), `verify` will report drift, and that is OK. You learn that the metric is noisy and you adjust your interpretation.

## Journaling

The decision is journaled before Git is touched. The journal includes the full decision, the measurements, the complexity metrics, and the run ID. If a crash happens after the decision but before finalization, `resume --run-id` can re-apply the recorded decision without re-evaluating.

This makes decisions idempotent and stable: the same candidate always gets the same decision, even if you run `resume` ten times.

## Metrics and their roles

Every metric you declare has a `kind`:

- `hard_gate` — Must pass. Failure discards immediately.
- `objective` — Drives the decision (exactly one per run). Compared exactly.
- `tie_breaker` — Recorded, never used by selection. Used only in reports and logs.
- `diagnostic` — Recorded, never used by selection. Useful for observations.

Only hard gates and the objective affect the decision. Metrics declared as `tie_breaker` or `diagnostic` are visible in the JSON report but do not influence whether a candidate is kept or discarded.

### Example manifest

Metrics are declared with a kind and direction:

```toml
[[evaluators.metrics]]
name = "paragraph_count"
kind = "objective"
direction = "minimize"

[[evaluators.metrics]]
name = "lighthouse_performance"
kind = "tie_breaker"
direction = "maximize"

[[evaluators.metrics]]
name = "bundle_bytes"
kind = "diagnostic"
direction = "minimize"
```

Decision logic:
1. Does `cta_present` pass? If no, discard.
2. Is `paragraph_count` better, worse, or tied? If tied, proceed.
3. Compare line count, then runtime.

The Lighthouse score and bundle bytes are recorded but do not affect the decision.

## Constraints in the manifest

`max_candidates`, `max_failures`, and `wall_clock_seconds` in the budget section are validated but not enforced by the CLI:

- The run does not stop after `max_failures` evaluator failures.
- The run does not stop after `wall_clock_seconds` elapsed.

These are conventions for humans or external orchestrators. If you need to enforce a time limit or failure budget, use an external timer or wrapper. The run will stop with `candidate_limit` after reaching `max_candidates`.

## When exact comparison works

Exact comparison is a choice with a cost. It suits:
- Byte counts (identical across runs of the same commit).
- Node counts (identical for deterministic traversals).
- Weighted defect counts (identical if the defects are deterministic).

It is wrong for:
- Single samples of noisy metrics (Lighthouse, benchmark timings, single-request latencies).
- Anything that jitters between runs.

For noisy metrics, emit a stable aggregate (mean, median, deterministic algorithm) or move the metric to a gate or tie-breaker role.

## Examples from the README

### Bundle bytes: exact and deterministic

```text
Baseline: 510,535 B
Best: 423,713 B
Reason: Bytes are deterministic. Same commit, same binary size.
Verification: Never drifts.
```

### Accessibility: exact gate

```text
Baseline: 5 axe violations + unresolved contrast nodes
Best: 0 nodes
Reason: Defects are deterministic if the same components render.
Verification: Matches if the same code path runs.
```

### SEO: exact but noisy scorer

```text
Baseline: 52.4 (local scorer on one post)
Best: 95.9
Candidate: 95.9 with tag change
Reason: Tied on score, but more lines changed.
Decision: Discard (tie_break_lines).
Note: Local scorer is deterministic for the same text, so no drift.
```

### Runtime: noisy

```text
Baseline: N/A
Candidate: 3 seconds (one run)
Verify: 2.8 seconds (different sample)
Reason: Wall-clock timing is noisy.
Decision: Drift detected. Evaluator is not stable.
```

## Decision always happens

Every candidate is always decided, even if something goes wrong. If the evaluator fails, the decision is `discard` with reason `failed_hard_gates` (evaluator failure is treated as a gate failure). The run does not hang or throw an error; it records the failure and moves on to the next candidate.

The only exception is if the CLI itself crashes before the decision is journaled. In that case, `resume --run-id` can apply the recorded decision or retry the evaluation.

The formal decision algorithm is in [decision-policy.md](../reference/decision-policy.md). A concrete example of the decision process appears in [first-loop.md](../tutorials/first-loop.md). For guidance on emitting stable metrics, see [write-an-evaluator.md](../how-to/write-an-evaluator.md).
