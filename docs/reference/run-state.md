# Run State Reference

A run lives in `.autoresearch/` inside the product repository. It contains worktrees, logs, journals, and measurements.

## Directory structure

```text
.autoresearch/
├── run.lock                             # Mutex lock; held while a command runs
├── runs/
│   └── run-<timestamp>-<pid>-<hash>/
│       ├── frozen/
│       │   ├── autoresearch.toml        # Frozen experiment contract
│       │   ├── program.md               # Frozen program
│       │   └── product-gate.md          # Optional, frozen when present
│       ├── identity.json                # Hash of frozen contract + environment
│       ├── environment.json             # Environment fingerprint
│       ├── journal.jsonl                # Append-only decision log
│       ├── baseline-worktree/           # Locked baseline evaluation worktree
│       ├── verifications/
│       │   └── verify-<nanos>-<pid>-<n>/
│       │       ├── verification.json    # Fresh evaluation measurements
│       │       └── verify-worktree/     # Locked verification worktree
│       └── artifacts/                   # Build output shared by baseline and candidates
│           ├── by-commit/
│           │   ├── <baseline-hash>/
│           │   └── <candidate-hash>/
│           └── verifications/           # Separate directory for verification builds
└── worktrees/
    └── run-<timestamp>-<pid>-<hash>/
        ├── candidate-000001/            # Locked detached worktree
        ├── candidate-000002/
        └── ...
```

## Lock files

`.autoresearch/run.lock` is a file-based mutex. Every command acquires it while working, so two commands never operate on the run at once. The lock file is created empty and stays empty; only the presence matters.

If a command crashes and leaves the lock held, delete the file and retry:

```bash
rm /path/to/product/.autoresearch/run.lock
autoresearch --repository /path/to/product status --run-id <id>
```

## Worktrees

Each baseline, candidate, and verification gets a locked worktree. Locked worktrees are not checked out anywhere in the main tree; they are purely detached branches.

**Baseline worktree.** The `baseline` command creates this and leaves it locked after evaluation. It contains the base commit and the frozen contract.

**Candidate worktrees.** The `run` command creates one per candidate and removes them after finalization, whether kept or discarded. Each contains one candidate commit.

**Verification worktree.** The `verify` command creates this and leaves it locked. It contains the kept commit.

Remove locked worktrees manually:

```bash
git -C /path/to/product worktree unlock .autoresearch/worktrees/run-<id>/candidate-000001
git -C /path/to/product worktree remove .autoresearch/worktrees/run-<id>/candidate-000001
```

Or bulk-remove with:

```bash
git -C /path/to/product worktree prune
```

## Journal

The journal is an append-only JSONL file that records every decision and recovery action. Each line is a separate `JournalEntry` object with a `kind` field identifying the entry type: `run_started`, `environment_captured`, `baseline_captured`, `candidate_prepared`, `candidate_evaluated`, `candidate_decision_recorded`, `candidate_finalized`, `recovery_action`, and `run_stopped`.

The journal is immutable and durable. Every command that changes state appends to it, and `resume --run-id` can re-apply any recorded decision without re-evaluating.

## Run ID format

Run IDs are:

```text
run-<unix-timestamp>-<process-id>-<8-char-hash>
```

Example:

```text
run-1789482005545-35729-7d116300
```

The three components are: `unix-timestamp` (millisecond-precision timestamp when baseline was started), `process-id` (process ID of the baseline command), and `8-char-hash` (first 8 characters of the baseline commit SHA).

Run IDs are unique across all machines and all times, with automatic collision handling. Two runs never produce the same ID, so files can be safely organized by run ID.

## Frozen identity

The frozen identity is a SHA256 hash computed from the frozen `autoresearch.toml` (as plain text), the frozen `program.md`, and the environment fingerprint (as a digest of environment variables and git config).

Two runs with identical manifests and environment produce identical frozen identity hashes. If either the manifest or the environment changes, the hash changes. This allows reproducible snapshots: you can audit whether the same contract ran with the same environment.

The hash is recorded in the run directory and printed at baseline:

```text
frozen identity: a48fb9b3c3e6002902fd1c0ae6a2e97b64d07e1e2ba026e159eaff5d53d832e1
```

## Artifact directory

The artifact directory is shared by baseline, all candidates, and verifications. It is where build output lives. Evaluators can write to `artifact_directory` from the request and read from it on subsequent runs.

The directory is keyed by commit hash, so:
- Baseline and verification both write to and read from their own commit subdirectory.
- Each candidate writes to its own subdirectory.

This allows warm caches: a cold build might take minutes, but a warm build (when the artifact directory already exists) can be seconds. The artifact directory is not gitignored and is not cleaned up automatically. Delete it manually when the run is complete and you are confident the measurements are stable.

To see what's in the artifact directory:

```bash
ls -la /path/to/product/.autoresearch/runs/run-<id>/artifacts/by-commit/
```

## Verification

Verification creates a separate worktree and a fresh artifact subdirectory. It does not share build cache with any candidate; it always builds cold. This prevents a shared warm cache from hiding non-determinism.

The verification report is written to:

```text
/path/to/product/.autoresearch/runs/run-<id>/verifications/verify-<nanos>-<pid>-<n>/verification.json
```

And includes:
- The kept commit's measurements (from the selection decision)
- The fresh measurements (from the verification run)
- The status (`matched`, `drifted`, or `failed`)

## Cleanup

After you merge the kept candidate and are confident in the measurements, you can delete:

1. The candidate worktrees:
   ```bash
   git -C /path/to/product worktree remove .autoresearch/worktrees/run-<id>/
   ```

2. The artifact directory (if you do not need it for reproducibility):
   ```bash
   rm -rf /path/to/product/.autoresearch/runs/run-<id>/artifacts
   ```

3. The entire run directory (if you are done with the run):
   ```bash
   rm -rf /path/to/product/.autoresearch/runs/run-<id>/
   ```

Do not delete an active run; `status`, `resume`, `verify`, and `report` all read from the run directory.

## Exporting

`export --run-id <id> --export-root /path/to/root` writes a report bundle to a new directory under the export root. This is useful for archiving runs or sharing reports outside the repository.

The bundle includes:
- `report.json` is the full JSON report
- `report.html` is the rendered HTML board
- `provenance.json` contains metadata (timestamp, run ID, frozen identity)

The run directory stays in `.autoresearch/` and is not affected by export.
