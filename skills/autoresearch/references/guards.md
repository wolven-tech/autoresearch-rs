# Statistical guards

Two guards for cases where a naive "keep it if the number went down" rule is wrong: a scalar that
is noisy, a corpus that can be overfitted, or both. A deterministic scalar needs neither.

- [How the CLI shapes both guards](#how-the-cli-shapes-both-guards)
- [Guard 1 — measured noise floor](#guard-1--measured-noise-floor)
- [Guard 2 — dev/holdout split](#guard-2--devholdout-split)
- [When neither applies](#when-neither-applies)

## How the CLI shapes both guards

The runner compares **one snapshot per candidate** against the current best and has no notion of
a noise band or a holdout set. So both guards live **inside the evaluator**, expressed as hard
gates and diagnostics. Seven facts decide how:

1. **A guard gate must pass at baseline.** At baseline `evaluated_commit` equals
   `baseline_commit`; short-circuit to pass there. A gate that fails at baseline still gives
   `evidence: captured` and `next: run` with exit 0; the failure shows only inside the snapshot.
   No candidate of that run can ever be decided: the submitted candidate's evaluators still run
   and are journaled, then `run` (and every later `resume`) exits 4 with ``baseline failed hard
   gate `<name>` `` and the run stays at `evaluate_candidate`, where `stop` refuses. So read the
   baseline snapshot's gates first and, **before any `run` call** (a bare prepare counts), `stop
   --run-id <id>`; then fix, commit and baseline again. Stop before you edit: while the checkout is
   dirty or HEAD is not the run's base commit, every command on that run fails until the checkout
   is clean at that commit again (`git switch --detach <base>`). If you already called `run`, that
   run cannot be stopped; leave it and baseline again.
2. **The comparison is against the incumbent, not the baseline.** The runner keeps a candidate
   only if it beats the current best (the last kept commit). The request's `baseline_commit` stays
   the original base forever; the incumbent is the candidate commit's parent: `/usr/bin/git -C
   <candidate_worktree> rev-parse HEAD^`. A gate measured against `baseline_commit` lets a
   candidate through that is inside the band of the incumbent.
3. **The evaluator has an empty environment** — no PATH, HOME or API keys — and the worktree as its
   working directory. Use absolute paths. An LLM-scored evaluator cannot read credentials from the
   environment, and declaring `requires_network = true` forces manual mode.
4. **The frozen identity does not cover evaluator code, its band or its corpus.** It covers the
   manifest (gate and metric names, kinds and directions; each evaluator's program, args and
   timeout), `program.md` and `docs/BET.md`. Commit the evaluator script outside `mutable_paths`
   and invoke it relative to the worktree (e.g. `program = "/usr/bin/python3"`, `args =
   ["eval/eval.py"]`) so the base commit fixes it; for a corpus outside the repo, pass its expected
   SHA-256 as a literal argument and fail on mismatch.
5. **`verify` demands exact equality** of every measurement, diagnostics and gate `detail`
   included, and runs in a different worktree (`verification-<commit>`) with a different
   `artifact_directory` (`artifacts/verifications/verify-…`, two levels below the run's shared
   one, so the sample cache misses). A noisy evaluator will usually `verify` as `drifted` (exit
   5), and so will any `detail` that embeds the working directory, a path or a timing, even for a
   deterministic evaluator. For a noisy objective that is the guard working as designed, not a
   regression: judge the fresh numbers against the band and say so in the write-up.
6. **The evaluator must leave its worktree byte-clean, ignored files included.** A benchmark that
   builds in the tree, or an interpreter writing `__pycache__`, fails baseline (`baseline worktree
   changed during evaluator`) or wedges a candidate (`candidate worktree must be clean before
   retention`). Keep build output and samples under `artifact_directory` or outside the repo; run
   Python with `-B` (macOS `/usr/bin/python3` happens not to write bytecode; others do). A timeout
   kills only the direct child, so an evaluator that fans samples out as subprocesses must reap
   them itself.
7. **A failure strands the candidate.** Any candidate evaluator failure — timeout, nonzero exit,
   protocol, validation — leaves it `prepared` at `evaluate_candidate`: no discard, `stop`
   refuses, and `run`/`resume` re-run the failing evaluator. `max_failures` and
   `wall_clock_seconds` are not enforced by the CLI, so "too expensive" is policed by you. Guard
   designs that can time out must be sized to fit before `baseline`.

Declare the gates in `hard_gates = [...]` on the `[[evaluators]]` entry and the diagnostics in
`[[evaluators.metrics]]` with `kind = "diagnostic"` and a `direction`; commit before `baseline`
(it refuses an untracked or dirty manifest, and any other untracked file — a new evaluator script
included — gets a run id allocated and then exit 4). Committing an evaluator or band fix moves
HEAD and locks every command out of older runs (`caller HEAD differs from frozen base commit`), so
`stop` the old run first, or `git switch --detach <its base_commit>` to reach it later.

## Guard 1 — measured noise floor

**Apply when the scalar is non-deterministic**: LLM output scored against ground truth, benchmark
wall-clock, anything sampled or timing-dependent.

The problem: re-running the *same* configuration against the *same* corpus produces a *different*
score. Keeping any change that improves the number records roughly half of the coin-flips as wins.
The ledger fills with noise and the thing under optimization drifts randomly — while looking, row
by row, like steady progress.

The fix, inside the evaluator:

1. **Sample R times per commit.** One `autoresearch baseline` call runs each evaluator once;
   calling `baseline` again does not replay, it freezes a new run. R = 5 is a reasonable default.
2. Report the median as the `objective` and the spread as a `diagnostic`.
3. Add a hard gate such as `clears_noise_band`: pass at baseline; for a candidate, fail unless its
   median beats the **incumbent's** median by more than the measured band. Cache each commit's
   samples in `artifact_directory`, keyed by commit — baseline and every candidate of a run share
   that directory, so the incumbent's samples are already there.
4. **Put the band on the row through the evaluator.** Only the runner writes the journal. Emit the
   band as a diagnostic and in the gate's `detail`. The call that evaluates a candidate (`run
   --json`, or `resume --json`) and `report` (JSON; `report --html` shows neither) show both for
   every candidate that reached a keep or discard decision, and the write-up's candidate row should
   repeat them.

Choosing the band is a judgement call — one standard deviation is permissive, two is conservative.
State which you used. What matters is that it is measured rather than assumed.

If R samples do not fit inside the evaluator's `timeout_seconds`, the evaluator is killed with
class `timeout` and no score is recorded. At baseline that ends the run (`evidence: failed`, exit
5, stop reason `baseline_evaluator_failure`). On a candidate the run cannot move past it: the
candidate stays `prepared` at `evaluate_candidate`, `stop` refuses, and every `run`/`resume`
re-runs that evaluator (exit 5 again unless the retry finishes in time). Size R to fit before
`baseline`. If you cut R because it does not fit or is too expensive, say so and treat the keeps of
that run as not guarded by a band. The CLI has no "unverified" row state (report states are
`prepared`, `decided`, `kept`, `discarded`; `verify` says `matched`, `drifted` or `failed`), so
record that caveat in the write-up. Do not silently fall back to a single measurement and present
it as if the band had been checked.

Runtime is also the last tie-breaker, and R samples multiply evaluator wall-clock. On an equal
objective the runner compares the candidate's changed-line count (added plus removed against its
parent) with the incumbent's, then dependency delta (always 0), then runtime. A candidate whose
line count matches a kept incumbent's is therefore kept or discarded on runtime alone, whatever its
diff contains. The baseline records zero complexity, so an equal-objective candidate against the
baseline is always discarded.

## Guard 2 — dev/holdout split

**Apply when overfitting is possible**: tuning a prompt, model, heuristic or ruleset against a
corpus that can be memorized rather than generalized from.

The problem: optimizing against a single set converges on something that scores well on *exactly
those cases* and worse on everything else. The loop reports steady improvement right up to the
point of deployment.

The fix:

1. Order cases deterministically — by capture time, or any stable key.
2. **Keep the holdout out of the repository.** The newest 30% is a good default: a time-ordered
   split also tests generalization to newer cases. `protected_paths` only rejects commits that
   change a path; it hides nothing. Every candidate worktree is a full checkout with protected
   files included, in manual mode the mutator is you editing that checkout, and a command-mode
   agent runs inside it with no filesystem sandbox (`program.md` is sent to it verbatim). Have the
   evaluator read the holdout from an absolute path outside the repo and check its SHA-256 before
   scoring. In manual mode, do not read that path during the run; run a command-mode agent only
   under an OS sandbox that denies it.
3. Score dev cases as the `objective`.
4. Add a hard gate such as `holdout_not_regressed`: pass at baseline; for a candidate, compare
   holdout against the incumbent (`HEAD^`), not against `baseline_commit`. Get the incumbent's
   holdout score by re-scoring its version (`/usr/bin/git -C <candidate_worktree> show
   HEAD^:<path>`) or from a cache outside the repository, keyed by commit and kept next to the
   holdout — never from `artifact_directory`. Improved on dev but regressed on holdout →
   **discard**, recorded as `"decision": {"disposition": "discard", "reason": {"reason":
   "failed_hard_gates", "names": ["holdout_not_regressed"]}}` (`names` lists every failed gate).
   Gates are checked before the objective, so the dev improvement never enters the decision. That
   row is one of the most valuable in the write-up: direct evidence of overfitting that would
   otherwise have shipped.
5. **Derive the corpus identity from the ordered membership** — a hash, passed as a literal
   argument and checked by the evaluator. Two candidates are only comparable against an identical
   case set, and a silently changed corpus invalidates every prior row.

**Never let a proposer see the holdout, including indirectly.** Put no holdout number anywhere in
the evaluator output. Gate `detail` and diagnostics are printed by `baseline`, `run --json`,
`resume --json`, `status --json` (baseline only), `report` (JSON, not `--html`) and `verify
--json`; they are written to `.autoresearch/runs/<run-id>/verifications/*/verification.json` and
copied into the `report.json` of every `export` bundle outside the repository. Each candidate's
full evaluator output, observations included, is journaled in
`.autoresearch/runs/<run-id>/journal.jsonl`. Never cache holdout scores in `artifact_directory` —
it sits inside the repo next to the worktrees. Once the holdout has informed a proposal, it is dev.

A holdout stored as a binary file (a model, a compiled ruleset) is fine outside the repo, but a
candidate whose diff touches a binary file inside the repo cannot be scored at all: it fails after
evaluation and strands the candidate.

## When neither applies

Byte counts, crate counts, static analysis, deterministic build output: the band is zero, repeated
measurement is wasted, and there is nothing to overfit. Measure once, compare directly, and skip
both guards. Ceremony that buys nothing makes the loop expensive enough that people stop running
it — which costs more than the rigour saves.
