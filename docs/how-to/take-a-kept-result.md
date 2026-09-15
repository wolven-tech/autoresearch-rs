# How To: Take a Kept Result

Once a candidate has been kept by autoresearch-rs, the local branch `autoresearch/<run-id>` points at the kept commit. Here's how to integrate that result into your repository.

## Before you merge

Run verification and generate a report to confirm the kept candidate:

```bash
autoresearch --repository /path/to/product verify --run-id <id>
```

Verify status must be `matched` (fresh measurements equal the selection). If it's `drifted`, your evaluator is noisy and you should investigate before merging (see [recover-a-run.md](recover-a-run.md)).

Generate a report:

```bash
autoresearch --repository /path/to/product report --run-id <id> --html > /tmp/report.html
```

Review the report in your browser. It shows baseline measurements, all candidates and their decisions, objective improvements, and changed paths with complexity metrics.

## Merging the result

You have three options: squash-merge, regular merge, or rebase.

### ### Option 1: Squash-merge (recommended)

This combines the candidate commit into a single new commit with your own message.

```bash
git -C /path/to/product merge --squash autoresearch/<run-id>
git -C /path/to/product commit -m "improve: brief description of the change"
```

This gives you a clean history with one commit per experiment, full control over the commit message, and easy-to-understand changesets.

### ### Option 2: Regular merge

This preserves the candidate commit history.

```bash
git -C /path/to/product merge --no-ff autoresearch/<run-id> -m "merge: autoresearch result"
```

This preserves the candidate commit and its timestamp, the original commit message (fixed by autoresearch-rs), and a merge commit showing the integration point.

### ### Option 3: Rebase

This replays the candidate on top of your current HEAD.

```bash
git -C /path/to/product rebase autoresearch/<run-id>
```

This is useful if your HEAD moved since baseline, but it risks conflicts if HEAD diverged. Resolve conflicts if any appear, then continue:

```bash
git -C /path/to/product rebase --continue
```

## After merge

Once merged, you can clean up the run state:

### ### Delete candidate worktrees

```bash
git -C /path/to/product worktree remove .autoresearch/worktrees/<run-id>/
git -C /path/to/product worktree prune
```

### ### Delete the artifact directory (optional)

If you no longer need the build cache:

```bash
rm -rf /path/to/product/.autoresearch/runs/<run-id>/artifacts
```

### ### Archive the run (optional)

Export the full run for records:

```bash
mkdir -p /tmp/autoresearch-archive
autoresearch --repository /path/to/product export --run-id <id> --export-root /tmp/autoresearch-archive
```

This creates a directory with `report.json`, `report.html`, and `provenance.json`.

### ### Delete the run directory

Once you've exported or archived:

```bash
rm -rf /path/to/product/.autoresearch/runs/<run-id>/
```

The local branch `refs/heads/autoresearch/<run-id>` can be deleted once you've merged:

```bash
git -C /path/to/product branch -d autoresearch/<run-id>
```

## Starting a new run

After merging, you can start a new run on the merged commit:

```bash
# Ensure HEAD is on the merged commit
git -C /path/to/product log --oneline -1

# Update and commit your contract if needed
git -C /path/to/product add autoresearch.toml program.md
git -C /path/to/product commit -m "chore: update autoresearch contract"

# Baseline for the next run
autoresearch --repository /path/to/product baseline
```

This creates a new run ID, a fresh `refs/heads/autoresearch/<new-run-id>`, and new worktrees. The old run directory stays in `.autoresearch/runs/` unless you delete it.

## Example workflow

```bash
# Initialize contract files if needed
autoresearch --repository /path/to/product init

# Baseline the experiment
autoresearch --repository /path/to/product baseline
RUN_ID="run-1789482005545-35729-7d116300"
autoresearch --repository /path/to/product run --run-id "$RUN_ID"       # open worktree
# Edit files in the worktree
autoresearch --repository /path/to/product run --run-id "$RUN_ID" --hypothesis "..." --json
# Candidate was kept

# Verify and review
autoresearch --repository /path/to/product verify --run-id "$RUN_ID"
autoresearch --repository /path/to/product report --run-id "$RUN_ID" --html > /tmp/report.html
# Review /tmp/report.html in a browser

# Merge the result
git -C /path/to/product merge --squash autoresearch/$RUN_ID
git -C /path/to/product commit -m "improve: remove redundant paragraphs"

# Clean up
git -C /path/to/product worktree remove .autoresearch/worktrees/$RUN_ID/
git -C /path/to/product branch -d autoresearch/$RUN_ID
rm -rf /path/to/product/.autoresearch/runs/$RUN_ID/

# Start a new experiment
autoresearch --repository /path/to/product baseline
```

## Handling conflicts

If your HEAD diverged from the base commit since baseline, a merge or rebase might produce conflicts. Resolve them in the normal way:

```bash
# For merge
git -C /path/to/product merge --squash autoresearch/<run-id>
# Resolve conflicts in your editor
git -C /path/to/product add .
git -C /path/to/product commit -m "..."

# For rebase
git -C /path/to/product rebase autoresearch/<run-id>
# Resolve conflicts
git -C /path/to/product add .
git -C /path/to/product rebase --continue
```

In most cases, squash-merge is safest because it treats the candidate commit as a single unit and avoids complex conflict resolution.

## Pushing the result

Once merged and tested, push to your remote:

```bash
git -C /path/to/product push origin HEAD:main
```

(Use your actual branch name instead of `main`.)

## What you should know

Verify refuses until a candidate has been kept (status and recovery action confirm this). When verify shows `drifted`, the measurements changed between selection and verification; check whether the evaluator is noisy or the environment varies.

Only `max_candidates` is enforced; `max_failures` and `wall_clock_seconds` are validated but never applied by the CLI. An objective tie against the baseline always discards, because baseline complexity is zero and tie-breaker metrics in the manifest do not affect selection decisions.

The entire repository must stay clean (no uncommitted or untracked files) through every command. Baseline and verification worktrees are not removed by autoresearch; you must clean them manually after merging.

Evaluators run in an empty environment (no PATH, HOME, LANG or user variables), so evaluator programs must use absolute paths. The --hypothesis flag is required to submit a candidate but is never stored in the journal or report.

## Read more

Start a new run from the merged commit by running `baseline` again. Read [explanation: how a run decides](../explanation/how-a-run-decides.md) to understand the selection policy that kept your candidate.
