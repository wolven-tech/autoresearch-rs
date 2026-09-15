# CLI Reference

autoresearch-rs is a command-line tool. All operations are subcommands with required and optional flags. Exit codes are 0 for success, 2 for usage, 3 for config, 4 for environment, and 5 for a failed evaluation or a verify mismatch.

## Common flags

| Flag | Purpose |
|---|---|
| `--repository PATH` | Absolute path to the product repository. Candidate worktrees are created under `.autoresearch/` inside this directory. Defaults to the current directory. |
| `--json` | Print output as JSON. Error responses go to stderr as `{"ok":false,"exit_code":N,"error":"..."}`. |
| `--help` | Print help for the subcommand. |

## init

Initialize an experiment by creating a template manifest and brief.

```bash
autoresearch --repository /path/to/product init
```

Creates the following files (does not overwrite if present):

- `autoresearch.toml`: The experiment contract.
- `program.md`: The experiment brief.

The template manifest uses placeholder binaries that will fail when you try to run baseline. The agent is `git status --short` and the evaluator is `git diff --numstat` (which does not output JSONL). Replace both with working programs and configure the objective, scope, and gates before running baseline.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error (e.g., missing `--repository`) |
| 3 | Config error (e.g., malformed manifest) |

## baseline

Freeze the contract and measure the base commit.

```bash
autoresearch --repository /path/to/product baseline
```

Requires:

- `autoresearch.toml` and `program.md` committed to the current HEAD
- `.autoresearch/` in `.gitignore`
- A completely clean working tree (no uncommitted changes or untracked files anywhere)

Creates:

- `.autoresearch/runs/<run-id>/`: The run directory
- `refs/heads/autoresearch/<run-id>`: The keep branch, starting at the base commit
- A locked baseline worktree

Outputs:

- `run: <run-id>`: The run identifier. Use this for all subsequent commands.
- `base commit: <hash>`: The frozen base commit.
- `frozen identity: <sha256>`: The hash of the frozen contract and environment.
- `evidence: captured`: The baseline measurements.
- `snapshot: {…}`: The baseline measurements as JSON.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error |
| 3 | Config error (manifest invalid) |
| 4 | Environment error (no git, missing `.autoresearch/` in gitignore, dirty tree, evaluator failed) |
| 5 | Evaluation failed (evaluator crashed or returned malformed JSON) |

## run

Open a candidate worktree, optionally commit and evaluate edits, and journal the decision.

```bash
autoresearch --repository /path/to/product run --run-id <id>
```

Without `--hypothesis`, prints the worktree path and exits with status `awaiting_mutation`. You edit files, then call `run` again with `--hypothesis` to submit.

```bash
autoresearch --repository /path/to/product run --run-id <id> --hypothesis "change description"
```

With `--hypothesis`, commits your edits in the worktree, evaluates them, decides whether to keep or discard, and applies the decision in Git.

Flags:

| Flag | Purpose |
|---|---|
| `--run-id <id>` | Required. The run identifier from baseline. |
| `--hypothesis <text>` | Required to submit. Arbitrary description of your change. Logged but not written to commits or reports. |
| `--json` | Print JSON instead of human text. Default is brief text (status and commit hash only). |
| `--mode command` | Run an agent inside the same call. Requires `--hypothesis`, `--allow-executable`, and the manifest must have `[authority] allow` empty. |
| `--allow-executable <path>` | Absolute path to a command-mode agent. Must match `agent.program` in the manifest after canonicalization. Required for command mode. |

### Submitting without changes

If no files changed in the worktree, the command exits 4: "candidate has no changed paths to commit".

### Submitting with protected-path or manifest changes

If your edit touches a path outside `mutable_paths`, a protected path, a symlink, a submodule, or changes dependency files (`Cargo.toml`, `Cargo.lock`, `package.json`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `bun.lockb`, `pyproject.toml`, `uv.lock`, `requirements.txt`, `go.mod`, `go.sum`) or any binary file, the command exits 4 with a description of the problem.

### Decision output

When `--json` is used, `decision.disposition` is `keep` or `discard`, and `decision.reason` is one of the following:

- `primary_improvement`: The objective got better.
- `primary_regression`: The objective got worse.
- `tie_breaker`: Objective tied; the field `field` names which complexity metric won: `changed_lines`, `dependency_delta`, or `runtime`.
- `no_improvement`: Objective tied, and all complexity metrics tied or favored the baseline.
- `failed_hard_gates`: One or more hard gates failed (names listed in the `names` field).

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success (candidate evaluated and decided) |
| 2 | Usage error |
| 3 | Config error |
| 4 | Environment error (dirty tree, no changes to commit, candidate touched a forbidden path, evaluator spawn failed) |
| 5 | Evaluation failed or decision failed |

## verify

Rerun evaluators on the kept commit.

```bash
autoresearch --repository /path/to/product verify --run-id <id>
```

Requires a candidate has been kept (a `keep` decision in the journal).

Reruns every evaluator with the kept commit as the evaluated commit. Compares the fresh measurements to the original decision. Status is `matched` if measurements are identical, `drifted` if measurements changed but the decision remains valid, or `failed` if evaluators crash.

Flags:

| Flag | Purpose |
|---|---|
| `--run-id <id>` | Required. |
| `--json` | Print JSON. |

Output includes:

- `status: matched` or `drifted`: Whether the fresh evaluation matched the selection.
- `evidence_path: <path>`: The path to the verification JSON file saved in the run directory.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success (verification succeeded) |
| 2 | Usage error |
| 3 | Config error |
| 4 | Environment error (no candidate kept yet) |
| 5 | Evaluation failed, or fresh measurements did not match the selection |

## status

Print the current state of a run.

```bash
autoresearch --repository /path/to/product status --run-id <id>
```

Flags:

| Flag | Purpose |
|---|---|
| `--run-id <id>` | Required. |
| `--json` | Print JSON. Without it, prints a human summary. |

Output includes:

- `recovery: <action>`: The next action to resume the run if something went wrong.
- `current commit: <hash>`: The current best kept commit (same as base if nothing kept yet).
- `base commit: <hash>`: The frozen base commit.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error |
| 3 | Config error |

## report

Generate a JSON report of the run.

```bash
autoresearch --repository /path/to/product report --run-id <id>
```

Combines the baseline, all candidates, and the journal into a structured report. Default is JSON. With `--html`, generates a script-free HTML board. Optionally imports external evidence via receipts.

Flags:

| Flag | Purpose |
|---|---|
| `--run-id <id>` | Required. |
| `--json` | Print JSON. This is the default. |
| `--html` | Render an HTML board instead. Cannot combine with `--json`. |
| `--evidence-root <path>` | Optional. Base directory for receipt sidecars. |
| `--receipt <path>` | Optional, repeatable. Relative path to a sidecar JSON file. Requires `--evidence-root`. |

The HTML board includes:

- Baseline measurements
- Candidate cards showing measurements, complexity, and decision
- Links to artifacts and verification evidence

Do not save the HTML inside the repository, because an untracked file makes every later `--run-id` command refuse. Redirect to a path outside:

```bash
autoresearch --repository /path/to/product report --run-id <id> --html > /tmp/report.html
```

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error |
| 3 | Config error (cannot combine `--html` with `--json`) |

## export

Write a report bundle to a directory.

```bash
autoresearch --repository /path/to/product export --run-id <id> --export-root /path/to/root
```

Writes the full report, HTML board, and provenance into a new timestamped subdirectory under the export root. Optionally includes run artifacts and imported evidence.

Flags:

| Flag | Purpose |
|---|---|
| `--run-id <id>` | Required. |
| `--export-root <path>` | Required. Absolute path to an existing directory outside the repository. |
| `--artifact <path>` | Optional, repeatable. Run-relative path to a file produced by evaluators. Must be under 4 MiB. Cannot be a log file or contain credential patterns. |
| `--evidence-root <path>` | Optional. Base directory for receipt sidecars. |
| `--receipt <path>` | Optional, repeatable. Relative path to a sidecar JSON file. |

Files written:

- `report.json`: The full JSON report.
- `report.html`: The HTML board (same as `report --html`).
- `provenance.json`: Metadata about the export (timestamp, run ID, frozen identity hash).
- `artifacts/`: Selected run artifacts (if any `--artifact` flags given).

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error |
| 3 | Config error |
| 4 | Environment error (export root missing or inside repository) |

## resume

Perform one recovery action from the journal.

```bash
autoresearch --repository /path/to/product resume --run-id <id>
```

Reads the journal and applies one recovery action based on the current state:

- If baseline evaluation is incomplete, capture baseline and run all evaluators.
- If a candidate is being prepared, verify the retain ref is ready.
- If a candidate is ready for evaluation, run evaluators and decide (replaying cached evaluator outputs rather than re-running them).
- If a decision was made but finalization is incomplete, apply the decision to Git.
- If the run is finished, print that no recovery is needed.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success (recovery action taken) |
| 2 | Usage error |
| 3 | Config error |
| 4 | Environment error |

## stop

Record a run cancellation.

```bash
autoresearch --repository /path/to/product stop --run-id <id>
```

Marks the run as stopped, preventing further candidates from being advanced. The run can still be resumed with baseline, but no more candidates can advance. This command only works when the run is idle between candidates; it refuses if baseline is pending, a candidate is active, or finalization is in progress.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage error |
| 3 | Config error |
| 4 | Environment error (run in an unsuitable state for stopping) |

## doctor

Check the repository, manifest, and evaluators without running anything.

```bash
autoresearch --repository /path/to/product doctor
```

Validates:

- Git is available and the repository is clean.
- `autoresearch.toml` exists and is valid TOML.
- `program.md` exists.
- The agent program can be found (bare names are resolved against your PATH; absolute paths are checked for existence).
- Every evaluator program can be found (same path resolution rules).

Gaps you should know:

- `doctor` resolves bare agent program names against your PATH, but evaluators run with an empty environment and never see PATH.
- `doctor` treats `program = "manual"` as a program to look up, so a manual-agent manifest reports not ready. Use `init` to create a working template, then replace the evaluator and set `program = "manual"` if you intend to edit candidates by hand.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | All checks passed |
| 2 | Usage error |
| 3 | Config error |
| 4 | Not ready (missing or invalid files, evaluator not found, etc.) |

## --version

Print the CLI version.

```bash
autoresearch --version
```

## --help

Print help for the CLI or a subcommand.

```bash
autoresearch --help
```

```bash
autoresearch run --help
```
