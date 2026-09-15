# How To: Recover a Run

If something goes wrong during a run (a crash, an unexpected failure, or an evaluator error), you can recover and continue. autoresearch-rs journals every decision, so recovery is idempotent and safe.

## Common recovery scenarios

### Evaluator crashed mid-run

The evaluator exited non-zero or timed out during candidate evaluation. The run is stuck waiting to finalize.

To recover, resume the evaluation:

```bash
autoresearch --repository /path/to/product resume --run-id <id>
```

`resume` executes the next recovery action from the journal. If the candidate was already committed, it re-evaluates using the frozen evaluators. If the evaluator output is incomplete or failed, the re-evaluation will replay or retry that evaluator. If the evaluator still fails, you need to fix it.

To fix the evaluator, edit your evaluator code to handle the failure:

```bash
autoresearch --repository /path/to/product resume --run-id <id>
```

Note: If the baseline itself failed during the `baseline` command, the run ends and cannot be resumed. Resume works only for failures during candidate evaluation.

### Candidate was committed but finalization failed

The candidate was committed but Git operations to move the branch failed. The run is inconsistent.

To recover, resume finalization:

```bash
autoresearch --repository /path/to/product resume --run-id <id>
```

`resume` reapplies the recorded decision without re-evaluating. If the decision was keep, the branch `refs/heads/autoresearch/<run-id>` is advanced. If the decision was discard, the orphaned commit is left for garbage collection.

### Working tree became dirty

You edited a file in the repository while the run was active. The next `run` call refuses because every command requires a clean repository and HEAD at the base commit.

To recover, clean the repository. Either revert your changes:

```bash
git -C /path/to/product checkout HEAD -- .
```

Or stash them:

```bash
git -C /path/to/product stash
```

Then retry the command:

```bash
autoresearch --repository /path/to/product run --run-id <id>
```

### HEAD moved during the run

You checked out a different branch or commit. Every command that touches the run state will refuse because it requires HEAD to match the base commit.

To recover, return to the base commit:

```bash
git -C /path/to/product checkout <base-commit>
```

The base commit is in the JSON output of `baseline` (pass `--json` to see it) and in the run state record.

Then retry the command:

```bash
autoresearch --repository /path/to/product run --run-id <id>
```

### Candidate was never submitted

You opened a worktree with `run --run-id <id>`, edited files in manual mode, but never called `run` again to submit. The worktree is still open and locked.

To recover, either submit the candidate by calling run again with your hypothesis:

```bash
autoresearch --repository /path/to/product run --run-id <id> --hypothesis "your change" --json
```

Or discard it by stopping the run:

```bash
autoresearch --repository /path/to/product stop --run-id <id>
```

Then delete the worktree:

```bash
git -C /path/to/product worktree remove .autoresearch/worktrees/<run-id>/candidate-NNNNNN
```

Replace NNNNNN with the candidate index shown by `status --run-id <id>`.

### Run timed out or was interrupted

You ran out of time or hit your candidate budget. The run is paused but not stopped.

To continue the run:

```bash
autoresearch --repository /path/to/product run --run-id <id>
```

This opens the next candidate if `max_candidates` has not been reached.

To stop the run cleanly:

```bash
autoresearch --repository /path/to/product stop --run-id <id>
```

This records a cancellation in the journal. Afterwards, `report` and `export` still work normally.

### Multiple commands tried to run at once

You ran two `autoresearch` commands on the same repository while one was still executing. The second command refuses with "could not acquire lock".

To recover, wait for the first command to finish. If the first command hung or crashed, check if a process is still running:

```bash
ps aux | grep autoresearch
```

If there is a stuck process, kill it:

```bash
kill <pid>
```

Then delete the lock file:

```bash
rm /path/to/product/.autoresearch/run.lock
```

Then retry your command:

```bash
autoresearch --repository /path/to/product status --run-id <id>
```

### Candidate worktree is locked but the run finished

After a crash, a worktree is left locked and no command cleans it up. You need to remove it to free space.

To recover, list the locked worktrees:

```bash
git -C /path/to/product worktree list
```

Unlock and remove each one:

```bash
git -C /path/to/product worktree unlock /path/to/worktree
git -C /path/to/product worktree remove /path/to/worktree
```

Or bulk-clean all stale worktrees:

```bash
git -C /path/to/product worktree prune
```

### Verify reported drift

`verify` reran the evaluators and got different measurements than the decision that kept the candidate. This means your evaluator is noisy or non-deterministic. Note: `verify` requires that a candidate was finalized as kept; it will refuse otherwise.

To diagnose the drift:

```bash
autoresearch --repository /path/to/product verify --run-id <id> --json
```

Look at `fresh_snapshot` vs `selection_snapshot`. If they differ, your evaluator is not stable. Common causes include sampling variations (e.g., Lighthouse samples vary by 2-3 points each run), timing variations (e.g., runtime benchmarks are noisy), non-deterministic ordering (e.g., sets, maps, or thread order), and floating-point precision (e.g., different rounding on different runs).

To fix the evaluator, have it emit a deterministic statistic (e.g., mean over multiple samples) instead of a raw sample. Move the noisy metric to `tie_breaker` or `diagnostic` role so it does not drive the decision (note that manifest tie_breaker metrics never affect selection, only the runner-measured complexity does). Characterize the noise first (e.g., with hyperfine for timings) to set realistic tolerances.

Then re-run the experiment with the stabilized evaluator.

## Step-by-step: Complete recovery from a crash

1. Identify the problem:
   ```bash
   autoresearch --repository /path/to/product status --run-id <id> --json
   ```

2. Clean up any stale locks:
   ```bash
   rm /path/to/product/.autoresearch/run.lock
   ```

3. Check the recovery action:
   ```bash
   autoresearch --repository /path/to/product status --run-id <id>
   ```

4. Apply the recovery action:
   ```bash
   autoresearch --repository /path/to/product resume --run-id <id>
   ```

5. Verify the run state:
   ```bash
   autoresearch --repository /path/to/product status --run-id <id> --json
   ```

6. If recovery succeeded, continue the run:
   ```bash
   autoresearch --repository /path/to/product run --run-id <id>
   ```

7. If there is a real problem (e.g., evaluator is broken), fix it and retry step 4.

## Advanced: Reading the journal

The journal is an append-only JSONL file. Read it to understand what happened:

```bash
jq . /path/to/product/.autoresearch/runs/<run-id>/journal.jsonl
```

Each line is a `JournalEntry` with an `event` field and a `sequence` number. Journal events include:

- `run_started`: Run identity and base commit recorded.
- `environment_captured`: Host environment fingerprint recorded.
- `baseline_captured`: Baseline measurements recorded.
- `baseline_failed`: Baseline evaluator failed and run stopped.
- `candidate_prepared`: New candidate worktree created.
- `candidate_evaluator_captured`: One evaluator completed for the candidate.
- `candidate_evaluator_failed`: One evaluator failed for the candidate.
- `candidate_decision_recorded`: Candidate was evaluated and decision made.
- `candidate_finalized`: Decision applied to Git (keep or discard).
- `run_stopped`: Run was stopped by operator.

The journal is immutable and durable, so if you understand the journal, you understand exactly what happened.

## Prevention: Run in isolation

To prevent accidental interference (dirty tree, HEAD moving, multiple commands), use a separate worktree:

```bash
git -C /path/to/product worktree add /path/to/autoresearch-worktree
cd /path/to/autoresearch-worktree
autoresearch --repository . baseline
```

This approach isolates the run from other work in the main repository tree, allowing you to keep working elsewhere while the run is active.

## After recovery

Once your run recovers, you can review the journal to understand what happened. Continue with the next candidate by calling `run --run-id <id>` again. Or finish the run by calling `stop --run-id <id>` followed by `report --run-id <id>`. For deeper understanding of how autoresearch makes decisions during evaluation, see [explanation: how a run decides](../explanation/how-a-run-decides.md).
