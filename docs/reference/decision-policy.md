# Decision Policy Reference

Every candidate is decided in a fixed order: gates first, then an exact compare of the objective, then line-count tie-breaking.

## Decision order

### 1. Hard gates

If a candidate fails any hard gate, it is discarded with reason `failed_hard_gates`. The objective is never read. The failed gate names are recorded.

Example:
```json
{"disposition":"discard","reason":{"reason":"failed_hard_gates","names":["cta_present"]}}
```

### 2. Objective compare

If all gates pass, the candidate's objective value is compared to the current best (baseline until something is kept, then the most recently kept candidate). The compare is exact with no epsilon and no noise tolerance.

When the candidate's objective is better in the direction declared in the manifest, the decision is keep with reason `primary_improvement`. When it is worse, the decision is discard with reason `primary_regression`. When the objective is exactly tied, the process proceeds to tie-breaking (below).

If the evaluator is noisy or non-deterministic, `verify` will report `drifted` when the fresh run differs from the selection. Move the metric to the tie-breaker role or add it to a gate that enforces stability.

### 3. Tie-breaking on changed lines

When the objective is exactly tied, the candidate's changed line count is compared to the current best's changed line count. Changed line count is the number of lines added or removed in the diff. Modified lines count as one line, and whitespace-only changes still count as line changes.

If the candidate has fewer lines changed, the decision is keep with reason `tie_break_lines`. If it has more lines changed, the decision is discard with reason `tie_break_lines`. If the line count is equal, the process proceeds to runtime tie-breaking (below).

### 4. Tie-breaking on runtime

When objective and line count are tied, the candidate's runtime (wall-clock milliseconds to evaluate it) is compared to the current best's runtime.

If the candidate is faster (fewer milliseconds), the decision is keep with reason `tie_break_runtime`. If it is slower (more milliseconds), the decision is discard with reason `tie_break_runtime`. If runtime is equal, the decision is discard with reason `no_improvement`.

Runtime is the elapsed time from when the evaluator started until it exited. It does not include Git operations, complexity measurement, or decision overhead.

## Tie against baseline

Ties against the baseline (a candidate with the same objective as the frozen base) always discard. The baseline's complexity is recorded as zero changed lines, so no candidate can achieve a "tie wins on fewer lines" against the baseline; ties against it always reach the runtime step, and baseline runtime is zero, so the tie-break always discards the candidate.

This prevents meaningless "improvements" like moving whitespace, which would not change the objective but would count as changed lines. A real improvement must change the objective or, at minimum, require fewer changed lines than any kept candidate.

## Tie against a kept candidate

A tie against a candidate that was previously kept is narrower: it survives if it changed fewer lines, or failing that ran faster. The line count and runtime are both recorded; exact equality on both results in discard with `no_improvement`.

Example timeline:
1. Baseline: paragraph_count = 2.0, changed_lines = 0
2. Candidate 1: paragraph_count = 1.0 (keep) → changed_lines = 3
3. Candidate 2: paragraph_count = 1.0 (tie against candidate 1) → if changed_lines < 3, keep with `tie_break_lines`

## Metrics in the decision

The manifest declares each metric's `kind`:

- **hard_gate** — Must pass. Failure ends the evaluation immediately.
- **objective** — Drives the decision (exactly one per run). Only the objective matters for keep/discard.
- **tie_breaker** — Recorded but never used by selection. Useful for context (e.g., the Lighthouse score that did not tie-break here).
- **diagnostic** — Recorded but never used by selection. Useful for observations and notes.

Metrics declared as `tie_breaker` or `diagnostic` are visible in reports and logs but do not affect the decision. Only hard gates and the objective are used.

## Journaling and idempotence

The decision is journaled before Git is touched. If a crash happens after the decision but before finalization, `resume --run-id` can apply the recorded decision again without re-evaluating. The journal entry includes the full decision and all measurements, so the decision is stable and idempotent.

## No epsilon, no noise tolerance

Exact comparison means small jitter shows up as a loss. If the evaluator is noisy (e.g., Lighthouse samples vary by 2-3 points), the first noisy candidate will tie and enter the line-count break, and `verify` will report a drift when the fresh run samples differently. The solution is to have the evaluator emit a stable statistic (a mean over several runs, a normalized score, or a deterministic algorithm) rather than a raw noisy sample.

## No repeats

If the same change produces different measurements on two runs, the second candidate will correctly show a drift in `verify`. There are no repeats built into the decision to handle this; if a metric jitters, that is a defect in the evaluator, not in autoresearch-rs.

## Constraints in the manifest are validated but not enforced

`max_failures` and `wall_clock_seconds` in the budget are checked by the CLI for validity (e.g., `max_failures` must be a non-negative integer) but are not enforced during the run. The CLI does not stop on failures or after elapsed time. Use an external timer or wrapper if you need to enforce these limits.

The `--hypothesis` text is required to submit a manual candidate but is thrown away: it is not written to the journal, the report, or commits. If a change's reason matters, write it down yourself (e.g., in a comment in the change or in your own notes file).
