# `autoresearch.toml` and evaluators

Schema version 1, as parsed by `autoresearch-config`. Commit it before `baseline`; it is frozen
per run. Invalid manifests make `doctor` report not ready (exit 4) and `baseline` exit 3.

## A manifest that works

This exact contract ran end to end on 2026-09-14 (doctor ready, baseline captured, one keep, one
gate-failed discard, `verify` matched):

```toml
schema_version = 1

[experiment]
name = "landing page content depth"

[experiment.objective]            # exactly one; an evaluator must report it as kind "objective"
name = "fixture_content_items"
direction = "maximize"            # "minimize" | "maximize"

[experiment.budget]               # all three must be > 0 and are frozen; only max_candidates stops a run
max_candidates = 3
max_failures = 2
wall_clock_seconds = 1800

[scope]
mutable_paths = ["site"]          # the only paths a candidate may change
protected_paths = []              # carve-outs inside a mutable root; rejects commits, hides nothing

[agent]                           # required; doctor checks the program exists, even in manual mode
program = "git"
args = ["status", "--short"]
timeout_seconds = 600

[[evaluators]]
id = "fixture_product_web"
hard_gates = ["cta_present"]      # every name must be reported, and pass, to keep

[evaluators.command]
program = "/absolute/path/to/product_web_loop_evaluator"   # absolute: evaluators get no PATH
args = []                                                   # literal, no shell
timeout_seconds = 60

[[evaluators.metrics]]
name = "fixture_content_items"
kind = "objective"                # objective | diagnostic | tie_breaker (recorded, never read)
direction = "maximize"

[authority]
allow = []                        # keep empty — see below
```

## Field rules

- **Paths.** `mutable_paths` is non-empty and repo-relative: no leading `/`, `..`, `.`, backslash
  or empty segment. A mutable root inside a protected root is rejected; a protected path inside a
  mutable root is a valid carve-out. `autoresearch.toml` and `program.md` are always added to
  `protected_paths`, so listing either as mutable fails validation. `docs/BET.md`, when present,
  must be tracked and clean at baseline, is copied to `frozen/product-gate.md` and checked against
  the base commit by every later command — but it is **not** protected: if a mutable root contains
  it, add it to `protected_paths` yourself.
- **Names.** Evaluator ids are unique, and every hard-gate and metric name across all evaluators
  shares one namespace.
- **Evaluator program.** Use an absolute path, or a repo-relative path to a committed executable
  (`tools/eval.sh`, resolved inside the worktree). Evaluators start with an empty environment (no
  PATH, HOME or CHROMIUM_PATH), so a bare name `doctor` finds on your PATH fails baseline with
  class `spawn` unless it lives in `/usr/bin` or `/bin`. The manifest has no way to declare
  environment variables. `doctor` ready does not mean baseline will work: it resolves bare names
  on your PATH and checks neither committed inputs, protocol compliance nor worktree hygiene.
- **`requires_network`** (optional on `[[evaluators]]`, default `false`). `true` is rejected unless
  `[authority] allow` lists `network` — and a non-empty `allow` makes command mode refuse, so an
  honestly declared network evaluator forces manual mode.
- **`[agent]`.** Executed only by `run --mode command`, which needs an absolute program resolving
  to the same file as `--allow-executable`, `[authority] allow = []` and `--hypothesis`. Literal
  args, no shell, working directory = the candidate worktree, environment only `LANG=C` and
  `TZ=UTC`, a `MutationRequest` JSON on stdin (candidate worktree, allowed files, frozen rubric,
  prior decisions — always empty today — hypothesis, cancellation id).
- **`[authority] allow`.** Valid names: `network`, `deployment`, `outreach`, `purchase`,
  `payment`, `production_write`, `permission_change`. The CLI has no per-run grant flag, any entry
  makes `run --mode command` refuse, and `network` only lets an evaluator declare
  `requires_network = true`. Nothing is isolated at runtime.
- **`market_evidence`.** The parser accepts `kind = "market_evidence"` in `[[evaluators.metrics]]`,
  but evaluator output may not carry it, so such a run can never baseline. Receipts come in
  through `report`/`export --evidence-root … --receipt …` instead.

## Metric kinds

| Kind | What the CLI does with it |
|---|---|
| hard gate (listed in `hard_gates`) | Must pass for a candidate to be kept; checked before the objective. Baseline does not enforce it: a failing gate is captured (exit 0), and the first candidate then fails after evaluation with `baseline failed hard gate` (exit 4); `resume` repeats that and `stop` refuses, so the run is stuck. Read the baseline snapshot; if a gate is false, `stop --run-id` before the first `run` (a bare prepare counts), fix, commit and baseline again |
| `objective` | Drives keep or discard against the current best |
| `diagnostic` | Recorded only; `verify` still demands exact equality |
| `tie_breaker` | Validated and recorded only; **never read by selection**. An equal objective is settled by runner-measured complexity against the current best: changed lines (this candidate's diff from its parent, compared with the diff that produced the current best), then dependency delta (always 0), then evaluation runtime. A tie with the all-zero baseline is always discarded; a tie with a kept candidate is kept when its diff is smaller (or the same size and faster), so a candidate that gains nothing can be kept |

## How evaluators run

- Every `[[evaluators]]` entry is run as a **JSONL protocol v1 subprocess**. There is no
  raw-command or Cargo mode in the CLI: `evaluate_command_gate` and `evaluate_cargo_check` exist
  only as `autoresearch-evaluator` library functions (and the Cargo one needs environment the CLI
  never passes). A plain exit-code command fails baseline with class `protocol`. To gate on cargo,
  write a JSONL evaluator that runs cargo by absolute path and sets its own environment: whatever
  cargo and rustup need (they get no HOME), and `CARGO_TARGET_DIR` outside the worktree, e.g.
  under `artifact_directory`.
- Working directory: the baseline, candidate or verification worktree — a full checkout of that
  commit, committed files only. **Leave it untouched, gitignored files included**: after every
  evaluator the runner checks `git status --untracked-files=all --ignored=matching`. A write fails
  baseline with class `validation` (`baseline worktree changed during evaluator`, exit 5), makes a
  candidate `run` exit 4 on every retry (`candidate worktree must be clean before retention: !!
  <path>`), and fails `verify` (`verification worktree changed during evaluator`). Write outputs
  only under `artifact_directory`; run interpreters that write bytecode with `-B`.
- Evaluators run in manifest order; the first failure stops the rest.
- stdout and stderr are each capped at 1 MiB (class `output_limit`); `timeout_seconds` is enforced
  (class `timeout`) and kills only the direct child; a nonzero exit fails even with a valid
  response (class `non_zero_exit`).
- Failures returned by the evaluator process (spawn, timeout, output limit, nonzero exit,
  protocol, reported, an envelope that fails validation) are redacted to `declared evaluator
  failed; raw process output withheld`; the class is kept. The runner's own checks keep their
  text — at baseline `baseline worktree changed during evaluator` and failed evaluator
  `snapshot_validation` (`declared evaluator outputs did not form baseline snapshot`); in
  `verify` `verification worktree changed during evaluator` and `fresh evaluator outputs did not
  form snapshot`; on a candidate `run` prints the Git or validation error directly, e.g. ``missing
  measurement `fixture_content_items` ``.
- To reproduce the runner's environment by hand, run `env -i <evaluator> < request.json` from the
  worktree (`.autoresearch/worktrees/<run-id>/baseline`).
- Run evidence lives under `.autoresearch/runs/<run-id>/`: `frozen/`, `identity.json`,
  `environment.json`, `journal.jsonl`, `artifacts/`, `verifications/`. `.autoresearch/` must be
  ignored, hold no tracked files and not be a symlink. Several runs can coexist in one repo, but
  every command on a run needs a clean checkout, untracked files included, with HEAD at that run's
  base commit; after committing anything else, `git switch --detach <base>` to work on the older
  run.

## Request (stdin)

One JSON line with exactly these fields, then stdin closes: `protocol_version` (1),
`evaluator_id`, `run_id`, `baseline_commit`, `evaluated_commit`, `candidate_worktree`,
`changed_paths`, `declared_environment` (always `{}` from the CLI), `artifact_directory`,
`cancellation_id`.

`changed_paths` is relative to the candidate's parent (the current best), not to
`baseline_commit`. `baseline_commit` is always the run's original base, even for later
candidates. The runner
compares against the current best, which is the candidate commit's parent: `/usr/bin/git -C
<candidate_worktree> rev-parse HEAD^`. Baseline and every candidate share one
`artifact_directory` (`.autoresearch/runs/<run-id>/artifacts`); `verify` gets a fresh
`artifacts/verifications/verify-…` subdirectory.

## Response (stdout)

Exactly one newline-terminated JSON line; logs go to stderr.

```json
{"protocol_version":1,"result":{"status":"success","output":{
  "evaluator_id":"fixture_product_web","run_id":"…","baseline_commit":"…","evaluated_commit":"…",
  "measurements":[
    {"kind":"hard_gate","name":"cta_present","outcome":{"passed":true,"detail":null}},
    {"kind":"numeric","name":"fixture_content_items","metric_kind":"objective","direction":"maximize","value":2.0}
  ],
  "observations":[],"artifacts":[],"warnings":[]}}}
```

(Shown on several lines for reading; the evaluator must print it as one line.)

- Echo `evaluator_id`, `run_id`, `baseline_commit` and `evaluated_commit` from the request.
- Report **every** gate and metric declared for that evaluator, with the declared kind and
  direction, and nothing undeclared, on every code path. At baseline a mismatch fails with failed
  evaluator `snapshot_validation` (class `validation`, exit 5). On a candidate `run` exits 4 (e.g.
  ``missing measurement `fixture_content_items` ``) after the outputs are journaled; `resume`
  replays those outputs even after the evaluator is fixed, and `stop` refuses, so the run is
  stuck. In `verify` a mismatch reports `failed` with `fresh evaluator outputs did not form
  snapshot`.
- A failing check is `"passed": false` (with an optional string `detail`), never a nonzero exit.
- Values must be finite. Unknown fields on the envelope, `output`, observations, artifacts or
  warnings fail closed; unknown fields inside a measurement or its `outcome` are silently dropped.
- Listed artifacts must already exist as regular, non-symlink files under `artifact_directory`.
- A failure response is `{"protocol_version":1,"result":{"status":"failure","failure":{"class":
  "reported","detail":"…"}}}`. Both fields are required and `reported` is the only class an
  evaluator may send; anything else is recorded as `protocol`.

Full protocol: `docs/evaluator-protocol.md` in autoresearch-rs (where it disagrees with this file
about adapters, the code wins).

## A known-good evaluator to start from

`apps/autoresearch-cli/tests/fixtures/product_web_loop_evaluator.rs` in autoresearch-rs: std-only,
reads `candidate_worktree` from the request, reports one hard gate (`cta_present`) and one
objective (`fixture_content_items`, the `<p>` count in `site/index.html`). Build it without cargo
(`<checkout>` is the absolute path of an autoresearch-rs checkout):

```sh
rustc +1.97.1 --edition=2024 -o /absolute/path/to/product_web_loop_evaluator \
  <checkout>/apps/autoresearch-cli/tests/fixtures/product_web_loop_evaluator.rs
```

Copy it and adapt the measurement. It panics when `site/index.html` is missing — a crash that
wedges a run — so make the copy report a failed gate instead. Do not start from
`crates/autoresearch-evaluator/examples/jsonl_evaluator.rs`: it returns a constant objective 1.25
(gate `tests`, metric `score`), so every candidate ties the all-zero-complexity baseline and is
discarded; it can never keep, and it needs the autoresearch crates, so plain `rustc` cannot build
it.

## The `init` starter

`init` writes `autoresearch.toml` and `program.md` without overwriting. The starter has objective
`changed_lines` (minimize), `mutable_paths = ["src"]`, agent `git status --short` and a `git diff
--numstat` evaluator. That evaluator does not speak JSONL, so baseline on the unmodified starter
fails with class `protocol` (exit 5): replace it before freezing. `init` does not add
`.autoresearch/` to `.gitignore`, which `doctor` and `baseline` both require.

## Web, SEO, GEO and portfolio packs

`examples/product-web/` and `examples/portfolio/` (UI, copy, performance, SEO, GEO, calculator,
mobile) are **manifest templates, not runnable CLI experiments**, and `examples/agents/` holds only
`[agent]` fragments (programs under `/opt/approved/bin/`), not manifests. As shipped, none of those
manifests baselines through `autoresearch baseline`: their `[agent] program = "manual"` fails
`doctor`; `autoresearch-web-evaluator` needs `CHROMIUM_PATH`, which the CLI never passes, and emits
only browser hard gates, so the product-web pack's `fixture_score` objective can never be produced;
the portfolio `REPLACE_WITH_…` placeholders fail on purpose. Copy table shapes from them and supply
your own absolute-path JSONL evaluators. Validation rules for `[web]` tables: `web.origin` must be a
loopback `http` origin; `viewports` must be exactly `[320, 390, 768, 1280]`; `web.geo` needs 1–16
facts and `max_passages` 1–32; redirects are capped at 5.
