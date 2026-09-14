---
name: autoresearch
description: Run a measured, falsifiable improvement loop with the autoresearch CLI (autoresearch-rs) — freeze a repository contract, capture a baseline against JSONL evaluators, try one hypothesis per candidate in an isolated Git worktree, and let the frozen lexicographic policy keep or discard it, with a journal, re-verification and an evidence report. Use when the user says "autoresearch", "run an autoresearch loop", "optimize X and measure it", "make this smaller/faster and prove it", "reduce bundle size", "improve this prompt/skill and measure", "tune this until it stops improving", or any request to iteratively improve something where "better" needs a number rather than a judgement call. Also use when a task is phrased as open-ended cleanup ("tidy this up") that would otherwise be done on vibes.
---

# Autoresearch

The engine is the `autoresearch` binary built from
[autoresearch-rs](https://github.com/wolven-tech/autoresearch-rs) (`apps/autoresearch-cli`). It
records the evidence: a frozen contract, one candidate per isolated worktree, a
gate-then-objective decision, an append-only journal, re-verification and a report. Your job is
the contract, the evaluator, the hypotheses and the write-up.

The CLI behaviour described here was verified against commit `3f2b989`, by reading the code and
running the recipe end to end. Where it differs from `docs/`, the code was followed. Update this
skill in the same change as any CLI behaviour it describes.

```
init → contract + JSONL evaluator → commit → doctor → baseline (read the snapshot)
  → { run → ONE uncommitted change in the worktree → run --hypothesis → record the row } × budget
  → stop (unless `run` already said stopped) → verify → report / export → only now: write-up in docs/
```

**Every claim of improvement must be a number a frozen evaluator actually produced.** If the
binary is missing, install it. Never fall back to an unmeasured loop and call it autoresearch.
If the task is one the CLI cannot run (see below), tell the user before anything else.

## Install or update

From a checkout of autoresearch-rs (`<checkout>` below is its absolute path):

```sh
cargo +1.97.1 install --locked --debug \
  --target-dir <checkout>/target \
  --path <checkout>/apps/autoresearch-cli
autoresearch --version
```

Use the toolchain pinned in the checkout's `rust-toolchain.toml` (1.97.1 at `3f2b989`): `cargo
install --path` takes its toolchain from the current directory, not from the target repo.
`--debug --target-dir` reuses the checkout's dev build instead of compiling a cold release build.
The CLI only orchestrates Git and evaluator processes, so a debug binary is enough.

Install the skill itself by linking this directory into your skills folder:
`ln -s <checkout>/skills/autoresearch ~/.claude/skills/autoresearch`.

## Flags, output, exit codes

Every command takes `--repository <repo>` (default `.`) and `--json`. **Use `--json` for
`baseline` and `run`**: in text mode `run` prints only `run:` and `status:`, plus `worktree:`,
`commit:` and `stop:` when they apply — never the snapshot, decision or index.

With `--json`, a command that produces a report prints one JSON document on stdout even when it
exits nonzero (`doctor` not ready, a failed baseline or `resume`, `verify` not `matched`). A
command that errors prints `{"ok":false,"exit_code":N,"error":"..."}` on stderr and nothing on
stdout. Usage errors are plain clap text. `report` prints JSON with or without `--json`;
`report --html` refuses `--json`.

- `0` ok — **including a baseline whose hard gate failed**, so read the snapshot
- `2` usage error
- `3` config: `baseline` rejects the manifest; `--mode command` without `--allow-executable` (or
  manual mode with one); `--receipt` without `--evidence-root`; `--html` with `--json`
- `4` environment or refusal: Git errors; a dirty checkout or uncommitted frozen inputs;
  `.autoresearch/` not ignored; `doctor` not ready (including an invalid manifest); every runner
  refusal — an `--allow-executable` that does not match `[agent] program`, command mode with a
  non-empty `[authority] allow`, a candidate that is empty, out of scope, touches a dependency
  manifest or a binary file, a candidate whose evaluator output omits, adds or mistypes a declared
  measurement (`missing measurement …`), a candidate scored against a baseline whose gate failed,
  `stop` during a candidate or on a finished run, run-state I/O (a misspelled `--run-id` gives `run
  state I/O failed`), another process holding `.autoresearch/run.lock`
- `5` failure: a baseline evaluator failed (including `snapshot_validation`), a candidate evaluator
  process failed, `verify` not `matched`, the CLI's own I/O (e.g. `init` into a missing directory)

## The rule that breaks most runs

From `baseline` until the run's last `verify`, `report` and `export`, **the caller checkout must
stay clean — untracked files included — with HEAD exactly at the run's base commit.** Every run
command checks it, `status` and `report` included:

- an untracked file → exit 4, `repository must be clean before a run: ?? <path>`
- any new commit → exit 4, `caller HEAD differs from frozen base commit`, on every command for
  that run

So: redirect reports outside the checkout (or under `.autoresearch/`, which is ignored); keep the
candidate ledger and any notes outside the repo; do other work in a separate worktree; write and
commit the `docs/` report only after the run is finished. To revisit a finished run, `git switch
--detach <base_commit>` in a clean tree. Never check out `autoresearch/<run-id>` in any worktree
(the runner refuses); inspect it with `git log` or `git diff <base> autoresearch/<run-id>`.

## What cannot run through the CLI today

- **Dependency and binary changes.** A candidate that adds, edits or deletes a file named
  `Cargo.toml`, `Cargo.lock`, `package.json`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`,
  `bun.lock`, `bun.lockb`, `pyproject.toml`, `uv.lock`, `requirements.txt`, `go.mod` or `go.sum`
  (at any depth), or any binary file, is refused after evaluation, and the run is stuck. A
  dependency-stripping loop, or tuning a model or ruleset stored as a binary, cannot use
  autoresearch — say so.
- **Tie-breaker metrics.** `kind = "tie_breaker"` is accepted and recorded; no decision reads it.
- **Failure and wall-clock budgets.** `max_failures` and `wall_clock_seconds` must be nonzero but
  are not enforced. Only `max_candidates` stops a run.
- **Exit-code or built-in Cargo evaluators.** Those adapters are library functions the CLI never
  calls. Every evaluator must speak JSONL.
- **Hypotheses.** `--hypothesis` is required but stored nowhere — not the journal, the report or
  the commit message.
- **Review.** No veto or approval step: a kept candidate is finalized immediately.

## Step 1 — Write the contract

```sh
autoresearch --repository <repo> init    # creates autoresearch.toml + program.md; never overwrites
```

`init` does **not** add `.autoresearch/` to `.gitignore`, and its sample `git diff --numstat`
evaluator cannot capture a baseline (it is not JSONL). Replace it. Field reference, the JSONL
protocol and a known-good evaluator: [references/manifest.md](references/manifest.md).

- **One objective.** `[experiment.objective]` names exactly one metric and its direction.
  Anything else that matters is a `diagnostic` — recorded, never selected on. Keep diagnostics
  deterministic: `verify` compares every measurement exactly.
- **The gate.** Requirements the work exists for become `hard_gates`. Optimizing bundle size
  without a gate converges on deleting the app. **Every gate must pass at baseline.** A gate that
  has never rejected anything is untested.
- **The budget.** `max_candidates` is the only stop the CLI enforces; set `max_failures` and
  `wall_clock_seconds` anyway (nonzero, frozen) and track failures and time yourself.
- **The boundary.** `scope.mutable_paths` is all a candidate may touch. `protected_paths` carves
  exceptions out of a mutable root; it only rejects commits and hides nothing.
  `autoresearch.toml` and `program.md` are always protected (listing either as mutable fails
  validation). `docs/BET.md`, when present, is hashed into the frozen identity and checked at the
  base commit, but it is **not** protected: if a mutable root contains it, add `docs/BET.md` to
  `protected_paths`, or a candidate can rewrite it and still be kept.
- **Evaluators** — JSONL v1 only. They run with an empty environment (no PATH, no HOME), in the
  worktree, which holds only committed files (no `node_modules`, `target/` or `.env`). Give the
  program as an absolute path, or as a repo-relative path to a committed executable
  (`tools/eval.sh`, resolved inside the worktree); a bare name is found only on `/usr/bin:/bin`, so
  anything in `/usr/sbin`, Homebrew or `~/.cargo/bin` fails with class `spawn`. **Never write into
  the worktree, not even gitignored files** (build output, logs, `__pycache__`): write only under
  the request's `artifact_directory`. A timeout kills only the direct child process.
- **A bad candidate must produce `passed: false`, never a crash.** A nonzero exit is an evaluator
  failure (exit 5), after which the candidate cannot be discarded or stopped. `run`/`resume` retry
  from the failed evaluator, which helps only when the cause is outside the candidate commit (an
  external binary, a transient timeout); a crash the candidate triggers repeats on every retry.
- **Pin the evaluator yourself.** The frozen identity does not hash evaluator code, so commit the
  evaluator inside the repo outside `mutable_paths` and invoke it relative to the worktree, or
  record the sha256 of an external binary in the write-up and do not rebuild it mid-run.
- **`[agent]`** is required even in manual mode, and `doctor` checks its program exists. Keep
  `init`'s `git status --short` unless you use command mode.
- **Authority.** `[authority] allow = []`. Valid names: `network`, `deployment`, `outreach`,
  `purchase`, `payment`, `production_write`, `permission_change`. The list is a ceiling, not a
  grant: the CLI has no per-run grant and isolates nothing. An evaluator with `requires_network =
  true` is rejected unless `network` is listed, and any non-empty `allow` makes command mode
  refuse. Get the user's explicit say-so before a run touches the network, deploys, writes to
  production or contacts anyone.

`program.md` is the mutator's brief: change only mutable paths, never the manifest, program,
evaluators or fixtures, one falsifiable hypothesis per candidate.

Add `.autoresearch/` to `.gitignore`, then **commit** `autoresearch.toml`, `program.md`,
`.gitignore` and the evaluator source. Before baseline, run the evaluator by hand the way the
runner will — `env -i <evaluator> < request.json` from a checkout, with a request line as in
[references/manifest.md](references/manifest.md) — on edge cases (missing file, empty file), and
confirm it leaves `git status --ignored` unchanged. A candidate cannot be discarded once its
evaluator fails, and a crash the candidate triggers repeats on every retry.

## Step 2 — Doctor and baseline

```sh
autoresearch --repository <repo> doctor            # Git, manifest, program.md, ignore rule, programs on YOUR PATH; runs nothing
autoresearch --repository <repo> --json baseline   # freezes inputs, evaluates the base commit once, prints run_id + snapshot
```

A `doctor` pass does not prove an evaluator will spawn — it resolves names on your shell's PATH,
the evaluator gets none. Only baseline proves it.

- **Dirty checkout:** an uncommitted or untracked frozen input (`autoresearch.toml`, `program.md`,
  `docs/BET.md`) is refused before any run id exists (`frozen inputs have uncommitted changes`,
  exit 4). Any other dirt allocates a run id and then exits 4 (`baseline run <id> could not
  evaluate: repository must be clean before a run: <status> <path>`). Remove the dirt **without
  committing** — move untracked files out, undo tracked edits; a commit moves HEAD off the base
  and locks the run out — then `resume --run-id <id>`. Do not start another baseline.
- **Evaluator failure (exit 5):** a process failure is redacted to `declared evaluator failed; raw
  process output withheld`, and only the class survives (`spawn`, `protocol`, `non_zero_exit`,
  `timeout`, `output_limit`, `validation`, `reported`). Two runner checks keep their own text:
  failed evaluator `snapshot_validation` (`declared evaluator outputs did not form baseline
  snapshot` — a declared gate or metric is missing, extra or mistyped, or a listed artifact is
  missing) and `baseline worktree changed during evaluator` (the evaluator wrote into the
  worktree). Either way the run is finished. Debug by running the evaluator by hand in
  `.autoresearch/worktrees/<run-id>/baseline`; fix, commit, baseline again (a new run).
- **Read the snapshot.** A failed hard gate is still `captured` with exit 0, but no candidate of
  that run can ever be decided. `stop --run-id <id>` **before any `run` call** (a bare prepare
  counts), then fix, commit and baseline again. If you already called `run`, that run cannot be
  stopped; leave it and baseline again.
- The frozen identity is a SHA-256 over the whole normalized manifest (experiment name, objective
  and budget; mutable and protected paths; the agent command; every evaluator's id, program, args,
  timeout, `requires_network`, gates and metrics; `[web]`; `[authority]`) plus the bytes of
  `program.md` and `docs/BET.md` when present. The environment fingerprint covers OS,
  architecture, runner version and declared program names — not tool versions or evaluator
  contents.

## Step 3 — Candidates (manual mode)

1. `autoresearch --repository <repo> --json run --run-id <id>` → `awaiting_mutation` with index and
   **worktree path**. Never edit the caller checkout.
2. Make **one** change in that worktree, inside `mutable_paths`, and **leave it uncommitted** —
   the CLI commits it. No ignored files left behind (build output, caches), no symlinks,
   submodules or nested repos, no dependency manifests, no binary files. An empty diff is refused.
3. `autoresearch --repository <repo> --json run --run-id <id> --hypothesis "<one falsifiable
   change>"` → `evaluated` with snapshot, decision and finalization.
4. **Record the row now**, outside the repo: index, commit, hypothesis, gates, objective, decision
   reason. The CLI cannot give the hypothesis back later.
5. Repeat until `run` returns `stopped` (`candidate_limit`), or `stop --run-id <id>` between
   candidates — after the baseline or an `evaluated` result, before the next `run` prepares a
   worktree.

**One hypothesis per candidate.** Attribution is the whole product: three changes in one candidate
make the decision say nothing about any of them.

**Selection** is lexicographic against the current best — the last kept snapshot, or the
baseline. Any failed hard gate discards (`failed_hard_gates` with the gate names). A better
objective keeps; a worse one discards. On an exactly equal objective the runner compares
complexity: this candidate's changed lines (added plus removed against its parent) with the diff
that produced the current best, then dependency delta (always 0 today), then evaluation runtime
(wall-clock, so noisy). A full tie discards. A kept candidate advances `autoresearch/<run-id>`
immediately and later candidates start from it.

**An equal objective can still be kept.** Against the untouched baseline (complexity all zeros)
it is always discarded. After a keep, a zero-delta candidate with a smaller diff than that keep —
or the same diff size and a faster runtime — is **kept** and advances the run branch. Record the
reason (`tie_breaker`/`changed_lines` or `tie_breaker`/`runtime`) and never report such a keep as
an improvement; a `runtime` keep is noise. To land a correctness fix that moves nothing, land it
outside the run; if it matters, add a gate the fix makes pass and baseline again.

**Stuck runs.** Once a candidate is prepared it cannot be discarded: `stop` refuses (`run can stop
only between candidates`) and `verify` refuses even when earlier candidates were kept. `run` and
`resume` retry the candidate. These fail the same way on every retry, even after the evaluator is
fixed: a dependency manifest or binary file, a failed baseline gate (`baseline failed hard gate`),
evaluator output that omits or mistypes a declared measurement (the journaled outputs are
replayed), an evaluator that wrote into the worktree (`candidate worktree must be clean before
retention: ?? <path>`), and a crash the candidate commit triggers. For these, fix the cause,
commit, and start a new baseline. A failure caused outside the candidate commit (an external
evaluator binary, a transient timeout) succeeds on retry once the cause is gone.

`status --run-id <id>` writes nothing (it still needs the clean checkout). `resume --run-id <id>`
follows the journal and refuses ambiguous Git effects. Never repair run state by hand. One process
owns a repository at a time: `run`, `stop`, `resume` and `verify` take `.autoresearch/run.lock`,
and a concurrent command in the same repo, even for another run, exits 4.

## Command mode

`--mode command --allow-executable /abs/path --hypothesis "…"` prepares, mutates and evaluates in
one call. `[agent] program` must be an absolute path resolving to the same file as
`--allow-executable`, and `[authority] allow` must be empty. The agent runs in the candidate
worktree with no shell, literal args, an environment of only `LANG=C` and `TZ=UTC`, and a JSON
`MutationRequest` (candidate worktree, allowed files, frozen rubric including `program.md`,
hypothesis) on stdin. With no HOME or credentials, provider CLIs cannot authenticate, and the
runner is **not** a sandbox — do not allowlist one without a trusted OS sandbox and the user's
explicit agreement.

## Step 4 — Verify, report, export, write up

```sh
autoresearch --repository <repo> stop --run-id <id>     # skip if `run` already returned `stopped`
autoresearch --repository <repo> verify --run-id <id>
autoresearch --repository <repo> report --run-id <id> --html > <path outside the checkout>
mkdir <absolute dir outside the repo>
autoresearch --repository <repo> export --run-id <id> --export-root <that dir>
```

Skip `stop` when `run` already returned `stopped` (`candidate_limit`): the run is finished, and
`stop` exits 4 with `run can stop only between candidates`.

- **`verify`** runs only when no candidate is in progress — between candidates, or after the run
  is finished (`stop` or `candidate_limit`) — and only if at least one candidate was kept;
  otherwise it exits 4 with `no finalized kept candidate is available for verification`. An
  all-discard run has nothing to verify; say so in the write-up. `verify` reruns the evaluators at
  the latest kept commit, in a separate `verification-<commit>` worktree with its own
  `artifact_directory`, and reports `matched` only when every measurement — diagnostics and gate
  `detail` included — equals the selection snapshot, so a `detail` that embeds a path or a timing
  drifts. `drifted` or `failed` exits 5. The result lands in
  `.autoresearch/runs/<id>/verifications/verify-*/verification.json` and is **not** in `report`
  or `export`, so cite its path and status. Report nothing as kept without `matched` (for a noisy
  objective see the guards).
- **`report`** is read-only; a redirect into the checkout creates an untracked file first and the
  command then refuses.
- **`export`** needs an existing absolute directory outside the repo, creates
  `autoresearch-<run-id>/` (`report.json`, `report.html`, `provenance.json`) and refuses to
  overwrite one. `--artifact` (repeatable) copies report-declared run artifacts;
  `--evidence-root` with `--receipt` imports commercial receipt sidecars.

**The deliverable is a document in the repo, written only now.** Put it in `docs/` (e.g.
`docs/reports/YYYY-MM-DD-<topic>-autoresearch.md`), link it from the docs index and commit it: net
delta against baseline first; one row per candidate from your ledger (index, commit, hypothesis,
gates, objective, decision reason — discards included); run id, base commit, frozen identity,
evaluator sha256, verify status and path; what is still open. Discards are data; a table of only
wins is a sales pitch.

**Cleanup.** The baseline and verification worktrees stay git-locked under
`.autoresearch/worktrees/`, and the `autoresearch/<run-id>` branch and any stuck candidate's
worktree stay behind too. Only after the write-up is committed and nothing will verify or report
on that run again: `git worktree unlock <path>`, then `git worktree remove <path>`.

## Rules the CLI cannot enforce

- **Do not improve the objective by deleting the requirement** — a supported platform, a
  deliberate architectural decision, an accessibility affordance. That is out of scope; record it
  as an open question for a human.
- **Keep ≠ ship.** A kept candidate advances the run branch only. Merging, deploying or making it
  the default is a human decision with the report in front of them; a human "veto" means not
  merging `autoresearch/<run-id>`. Never auto-promote.
- **Report what was measured.** An evaluator failure is a typed failure, not a zero and not an
  estimate.
- **Lab numbers are not market evidence.** Traffic, clicks, impressions, praise, lab scores and
  synthetic telemetry never move a product decision.

## Noisy or overfittable objectives

Load [references/guards.md](references/guards.md) when the objective is non-deterministic (LLM
output, timings) or tunable against a corpus that can be memorized. The runner compares one
snapshot per candidate and `verify` demands exact equality, so the noise floor and the holdout
live inside the evaluator. Deterministic objectives (byte counts, changed lines, static analysis)
need neither.
