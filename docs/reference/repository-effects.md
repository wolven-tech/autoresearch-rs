# Repository Effects Reference

autoresearch-rs modifies the Git repository in specific, predictable ways. Understanding these effects prevents conflicts and helps you manage runs safely.

## Before baseline

Start by initializing the configuration and freezing the baseline state. Before you run baseline, the working tree must be clean: no uncommitted changes outside `.autoresearch/`, no untracked files outside `.autoresearch/`, and `autoresearch.toml` and `program.md` must be committed.

Preconditions checked by baseline:
- `autoresearch.toml` and `program.md` are committed and match the manifest being used.
- No uncommitted changes exist (untracked files in `.autoresearch/` are okay).
- No untracked files exist outside `.autoresearch/`.
- `.autoresearch/` is listed in `.gitignore`.
- Your HEAD is on the commit you intend to evaluate.

Initialize a new experiment with:

```bash
autoresearch init
```

This creates `autoresearch.toml` and `program.md` from templates. You must edit these files, commit them, and then run baseline.

## What baseline does

Freeze the baseline state and prepare for evaluation. Baseline evaluates all declared evaluators against your current HEAD and records the measurements.

Create a baseline with:

```bash
autoresearch baseline
```

This command creates:
- A new local ref `refs/heads/autoresearch/<run-id>` pointing at your current HEAD (the base commit).
- A directory `.autoresearch/runs/<run-id>/` with the frozen manifest, program, identity, environment and journal.
- A directory `.autoresearch/worktrees/<run-id>/baseline/` with a locked detached checkout where evaluators run.

Baseline does not:
- Modify your working tree.
- Move your HEAD.
- Push anything.
- Create new commits (evaluators may create temporary files that are cleaned up).

If baseline succeeds, all evaluators pass and the run is ready for candidates.

## What run does

Submit candidates for evaluation. Each call to `run` either prepares a candidate for manual editing or evaluates an already-prepared one.

Manual mode: prepare a candidate for you to edit:

```bash
autoresearch run --run-id <run-id> --mode manual
```

Manual mode: evaluate your edits:

```bash
autoresearch run --run-id <run-id> --mode manual --hypothesis "description of change"
```

When you submit a candidate with `run --run-id <id> --hypothesis "..." `:
- The candidate worktree's uncommitted changes are committed with author set to your git config.
- The commit is made without hooks (`--no-verify`).
- The commit message is fixed (not your hypothesis; hypothesis is logged but not committed).
- The commit is orphaned (not on any branch yet).

After evaluation and decision, one of two outcomes occurs:
- On **keep**: `refs/heads/autoresearch/<run-id>` advances to the candidate commit via compare-and-swap.
- On **discard**: The candidate commit stays orphaned and is eventually garbage-collected.

In both cases, the candidate worktree is removed (locked worktrees are cleaned up).

Your HEAD and working tree are never modified by `run`. You can work on other branches while a run is active, but every `--run-id` command refuses if your current HEAD differs from the frozen base commit.

## Git refs you cannot push

The ref `refs/heads/autoresearch/<run-id>` is a local branch created for each run and intended to stay local only. While Git does not prevent you from pushing it, doing so would defeat the purpose of the isolated evaluation framework.

The run state is not meant to be shared because:
- Each run holds a locked worktree with uncommitted changes from candidate evaluation.
- Pushing the ref would make those worktrees visible to other users without the ability to use them (they are locked to the local machine).
- The journal and frozen state are machine-specific and contain local paths and timestamps that do not transfer meaningfully.

If you want to integrate a kept candidate into a shared branch, do not push the `autoresearch/<run-id>` ref. Instead, squash-merge the result onto your main branch:

```bash
git checkout main
git merge --squash autoresearch/<run-id>
git commit -m "description of change"
git push
```

This preserves the candidate's changes while replacing the fixed autoresearch commit message with your own, resulting in cleaner shared history.

## What verify does

Verify that a kept candidate's measurements are stable and reproducible. Run verify after you have kept a candidate and want to confirm its results.

Verify a kept candidate with:

```bash
autoresearch verify --run-id <run-id>
```

This re-runs all evaluators in a fresh worktree and compares measurements against the original baseline and candidate runs. Status is either `matched` (exact equality), `drifted` (small differences), or `failed` (incompatibility or error).

Verify creates:
- `.autoresearch/runs/<run-id>/verifications/verify-<nanos>-<pid>-<n>/` directory with verification measurements.
- `.autoresearch/worktrees/<run-id>/verification-<commit>/` locked worktree where evaluators run.

Verify does not modify Git refs or create commits. The journal and retained ref remain unchanged.

## What report does

Rebuild or display the full evaluation report without running evaluators again. The report is always printed as JSON:

```bash
autoresearch report --run-id <run-id>
```

Generate an HTML board for viewing in a browser:

```bash
autoresearch report --run-id <run-id> --html
```

The report reconstructs the frozen inputs and the journal into a complete RunReportV1 with all measurements, decisions, and metadata.

## What export does

Export the report and evidence outside the repository for sharing or archival. Export is read-only and does not modify any state:

```bash
autoresearch export --run-id <run-id> --export-root /path/outside/repo
```

`export` writes three files to `<export-root>/autoresearch-<run-id>/`:
- `report.json`: the full JSON report.
- `report.html`: the HTML board.
- `provenance.json`: metadata about the export.

It does not modify the repository or run directory.

## Candidate commits

Candidate commits are created during evaluation and represent the changes evaluated in one `run` command. Each candidate is assigned a zero-padded number (000001, 000002, etc.) incrementing across the run.

Candidate commit properties:
- Fixed message format: `autoresearch(<run-id>): candidate NNNNNN`.
- Author is your `user.name` and `user.email` from git config.
- Made with `--no-verify` so pre-commit and commit-msg hooks do not run.
- Created in the isolated candidate worktree `.autoresearch/worktrees/<run-id>/candidate-NNNNNN`, not in your checked-out tree.
- Parent commit is either the baseline commit (for the first candidate) or the most recently kept candidate.

After evaluation, the decision to keep or discard determines the candidate's fate:
- **Keep**: The `refs/heads/autoresearch/<run-id>` ref advances to point to this candidate commit. The commit becomes part of the run's decision history and is the starting point for the next candidate.
- **Discard**: The commit remains orphaned in the repository and is eventually garbage-collected unless you reference it.

If you want to examine a kept candidate commit after the run completes, check it out:

```bash
git log autoresearch/<run-id> -1
git show autoresearch/<run-id>
```

To integrate the candidate into your main branch, squash-merge it:

```bash
git merge --squash autoresearch/<run-id>
git commit -m "your message"
```

This preserves the candidate's changes but replaces the fixed commit message with your own.

## Constraints on candidates

A candidate commit is refused during creation if it:
- Touches any path outside `scope.mutable_paths`.
- Touches any path in `scope.protected_paths`.
- Touches a symlink, submodule, or nested .git directory.
- Leaves ignored untracked files in the working tree (the evaluator must clean up).

A candidate cannot be decided if it:
- Contains a binary file.
- Modifies `Cargo.toml`, `Cargo.lock`, `package.json`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `bun.lockb`, `pyproject.toml`, `uv.lock`, `requirements.txt`, `go.mod`, or `go.sum`.

If a refusal occurs during commit creation, the command exits 4 with a description. If a candidate cannot be decided due to binary or manifest changes, complexity measurement fails after evaluators run and the candidate is discarded.

## Working tree cleanliness

From baseline until you are done with the run, your checked-out working tree must stay clean:
- No uncommitted changes.
- No untracked files outside `.autoresearch/`.
- Your HEAD must stay on the base commit.

Every `--run-id` command checks this and refuses if the tree is dirty. If you need to work on something else, use a different worktree or clone.

## Recovering from a crash

If the CLI crashes or is killed mid-run, the run directory is left locked by `.autoresearch/run.lock`. The lock file contains the run ID and process ID of the lock holder.

Recovery requires three steps. First, delete the stale lock file:

```bash
rm /path/to/product/.autoresearch/run.lock
```

Second, check the run state to understand where the run stopped:

```bash
autoresearch --repository /path/to/product status --run-id <id>
```

The status output shows the recovery action: start_run (frozen inputs not captured), capture_baseline (baseline evaluation in progress), prepare_candidate (awaiting candidate edits), evaluate_candidate (awaiting evaluation completion), finalize_candidate (awaiting decision application), or finished (run complete).

Third, apply recovery actions by resuming the run:

```bash
autoresearch --repository /path/to/product resume --run-id <id>
```

Resume executes exactly one recovery action based on the current state. It replays already-journaled evaluator outputs instead of re-running them, so measurements remain consistent with the earlier invocations. Repeat `resume` until it reports no recovery action is needed, meaning the run has advanced to the next waiting point or completed.

If you cannot determine the state safely, running `status` multiple times is safe and read-only. Status never modifies any Git state or the run directory.

## Candidate worktree cleanup

Candidate worktrees are removed after finalization, which occurs after the decision (keep or discard) has been made and applied to Git. If a command crashes before finalization completes, the worktree is left behind in a locked state with the `.autoresearch/run.lock` file present.

Clean up a stale candidate worktree manually. First, delete the lock file if it still exists:

```bash
rm /path/to/product/.autoresearch/run.lock
```

Then unlock and remove the worktree:

```bash
git -C /path/to/product worktree unlock .autoresearch/worktrees/<run-id>/candidate-000001
git -C /path/to/product worktree remove .autoresearch/worktrees/<run-id>/candidate-000001
```

Or remove all stale worktrees at once:

```bash
git -C /path/to/product worktree prune
```

The `worktree prune` command removes all orphaned, stale, and locked worktrees that are no longer in use. It is safe to run even if no worktrees need cleanup.

Note that baseline and verification worktrees are not automatically removed after their respective operations complete. You may want to keep them for debugging or re-running evaluation, but you can manually remove them using the same `worktree remove` process if disk space is a concern.

## Multiple runs

Multiple runs can exist at once, with separate run IDs, refs, and directories. Each run is isolated and does not interfere with the others. However:
- Only one command per run can execute at a time (the lock file ensures this).
- Your working tree's HEAD must match the base commit that was frozen when you started the run. Switching to a different run requires checking out that run's base commit.
- If you switch to a different run's base commit, verify the old run is finished and delete any candidate worktrees from the old run before starting work on the new one.

## Merging when ready

To integrate the kept candidate into your main branch:

```bash
# See what's in the kept commit
git -C /path/to/product log autoresearch/<run-id> -1

# Merge it with a custom commit message
git -C /path/to/product merge --squash autoresearch/<run-id>
git -C /path/to/product commit -m "your message"

# Or rebase
git -C /path/to/product rebase autoresearch/<run-id>
```

The squash-merge approach preserves the candidate's changes while replacing the fixed `autoresearch(<run-id>): candidate NNNNNN` message with your own, making the shared history clean and readable.

After merge and push, you can delete the run's worktrees and artifact directory to free space (see [run-state.md](run-state.md) for details).

## Common refusal messages

The CLI exits with code 4 when preconditions are violated. Understanding these messages helps you fix the underlying issue:

Working tree is not clean: Run `git status` and either commit your changes or stash them. Untracked files in `.autoresearch/` are okay, but anything outside `.autoresearch/` must be tracked and unmodified.

`.autoresearch/` is not ignored: Add `.autoresearch/` to your `.gitignore` file at the repository root and commit it. This allows autoresearch to create and manage state files without cluttering version control.

Candidate HEAD differs from parent: In manual mode, you must not commit changes yourself before calling `run --hypothesis`. The worktree should be dirty with your edits as uncomitted changes. Call `run` again to commit and evaluate those edits.

Candidate touches mutable paths outside scope: The candidate edits files not listed in `scope.mutable_paths`. Check your `autoresearch.toml` configuration and expand the mutable paths list if needed, or undo edits to protected files.

Evaluator failed: Run `autoresearch status --run-id <id>` and then `autoresearch resume --run-id <id>` to inspect the failure output in detail. Common causes include evaluator script not executable, evaluator expecting an input file that is not present, or evaluator crashing during execution.

## Frozen state and decision policy

When baseline succeeds, the run's inputs are frozen in `.autoresearch/runs/<run-id>/frozen/`. This frozen copy includes your `autoresearch.toml`, `program.md`, and optionally `docs/BET.md`. These files are byte-identical to what was committed at the time of baseline and cannot be modified during the run.

The decision process compares each candidate against the current best (initially the baseline, then the most recently kept candidate). Comparison rules:

- Hard gates must pass. Any candidate that fails a hard gate is discarded immediately.
- The objective metric is compared for improvement, regression, or tie.
- On improvement: candidate is kept.
- On regression: candidate is discarded.
- On a tie: complexity (lines changed, runtime) decides. If all are equal, the candidate is discarded because the baseline has zero complexity.

Once kept, a candidate becomes the new current best and parent for the next candidate. This decision history is permanent and cannot be undone without manually editing the journal, which is not recommended.

## Run directory structure

The `.autoresearch/` directory contains all run state. Understanding its structure helps you recover from crashes and manage disk space:

Lock file `.autoresearch/run.lock` is a JSON document with the current lock holder (run ID, process ID, and acquisition time). Only one process can hold the lock at a time. If the process crashes, you can safely delete the lock file and retry with `resume`.

Each run creates `.autoresearch/runs/<run-id>/` containing:
- `frozen/` subdirectory with byte-copies of `autoresearch.toml`, `program.md`, and optional `docs/BET.md`.
- `identity.json` metadata about the run: run ID, base commit, operator user name and email, and the frozen manifest.
- `environment.json` system environment info: host OS and architecture, runner version, program names.
- `journal.jsonl` newline-delimited JSON recording every action and decision.
- `artifacts/` for evaluator outputs (measurements, observations, and output files).
- `verifications/` subdirectory for each verification run and its measurements.

Worktrees are in `.autoresearch/worktrees/<run-id>/`:
- `baseline/` the locked baseline worktree where evaluators ran initially.
- `candidate-NNNNNN/` locked candidate worktrees created and removed as candidates are evaluated.
- `verification-<commit>/` locked verification worktrees if verification was run.

The journal records decisions as JSONL and is the source of truth for which candidates were kept or discarded. It includes the full decision details: objective values, gate passes/failures, complexity measurements, and the decision reason.

## State consistency

Autoresearch maintains consistency between the Git state and the run directory through its lock file and journal. The lock ensures only one process modifies state at a time. The journal records every action before Git effects, so if a crash occurs mid-operation, recovery can replay the decision without re-evaluating.

The `refs/heads/autoresearch/<run-id>` ref always points to either the base commit (initial state) or the most recently kept candidate. It never points to a discarded candidate or an intermediate state. On KEEP, the ref advances via `git update-ref` with compare-and-swap semantics: the write only succeeds if the ref still points to the parent commit, preventing race conditions if multiple processes accidentally try to modify the same run.

Worktrees are locked exclusively to prevent concurrent modification. A locked worktree cannot be checked out in another worktree or deleted without first unlocking it. The lock reason is recorded and includes the run ID and operation type (baseline, candidate evaluation, or verification).

Candidate and verification worktrees are created from explicit commits (the parent commit for candidates, the kept candidate for verification). They are detached worktrees pointing to that commit, not tracking any branch. This isolation ensures evaluations cannot be affected by branch changes or concurrent work on other branches.

## What happens when you abort a run

If you stop a run mid-way (for example, by deleting the lock file and not running resume), the run directory remains with its current state. Baseline and verification worktrees are never automatically cleaned up. Candidate worktrees created before the abort are left behind locked and must be manually removed or cleaned with `worktree prune`. The run ref stays at whatever commit it had reached, allowing you to resume later or manually inspect intermediate candidates if needed.

Aborting a run does not remove any commits. Discarded candidate commits remain in the repository as unreferenced objects until garbage collection runs (which can take days or weeks). If you later delete the run directory without first running status or resume, those orphaned commits become unrecoverable except through manual ref inspection.

## Interaction with other Git operations

Your checked-out working tree and the run state are independent. While a run is active, you can use other Git worktrees to work on different branches without affecting the run. The only constraint is that your primary worktree's HEAD must remain at the frozen base commit for that specific run.

You can have multiple runs active at once (each with its own run-id and directory). Each run holds an exclusive lock at the repository level (`.autoresearch/run.lock`), so only one `baseline`, `run`, `verify`, or `resume` command can execute at a time, even across different runs. However, you can run `status`, `report`, or `export` commands in parallel without acquiring the lock because they are read-only.

Switching between runs requires a clean working tree and checking out each run's respective base commit. Use `git checkout <base-commit>` or a separate worktree for each run. Do not use the same primary worktree for multiple concurrent runs.

Cherry-picking or rebasing candidate commits outside of autoresearch is possible but not recommended. The commit message format `autoresearch(<run-id>): candidate NNNNNN` is stable but the parent commit relationships are not guaranteed to be linear if you manually rearrange history. Use squash-merge instead when integrating into your main branch.

Pushing the autoresearch ref and its commits is possible but breaks isolation. If you do push, other users cannot use those worktrees and the run state becomes distributed across machines, defeating the purpose of frozen, local evaluation. Always squash-merge onto a shared branch instead.

## Disk space and cleanup

Runs can consume significant disk space depending on the number of candidates and the size of evaluated artifacts. Each candidate worktree is a full checkout of the repository plus any artifacts generated during evaluation. Baseline and verification worktrees persist after their respective operations complete and should be cleaned up if disk space is constrained.

Delete a worktree manually:

```bash
git -C /path/to/product worktree remove .autoresearch/worktrees/<run-id>/baseline
git -C /path/to/product worktree remove .autoresearch/worktrees/<run-id>/verification-<commit>
```

The run directory itself (`.autoresearch/runs/<run-id>/`) is kept indefinitely as a record of the run. You can delete it manually after finishing with a run, but the decision history will be lost. If you later want to re-run the same evaluation, you will need to create a new run.

The `git gc` command will garbage-collect unreferenced candidate commits and free space. This can take considerable time on large repositories with many discarded candidates. Discarded candidate commits may not be immediately reclaimed even after running `git gc`, as cleanup depends on your specific Git configuration and system load patterns.

For large-scale experimentation over many concurrent runs, consider a separate clone repository as the evaluation target to keep main development clean and responsive.
