# Phase 2 Plan 2: Run Branch and Candidate Worktree Summary

**Each run now owns one retained current-best ref while every candidate starts in a detached, locked worktree outside caller Git state.**

## Accomplishments

- Added core-owned `RunId`, `RunWorkspace`, `CandidateWorkspace`, and `CandidateWorkspaceManager` port.
- Added canonical `refs/heads/autoresearch/<run-id>` naming and deterministic `candidate-<index>` worktree identity.
- Added lock-bound Git adapter that atomically creates or resumes a retained run ref without checking it out.
- Added detached candidate worktrees under ignored `.autoresearch/worktrees/` and Git worktree locks against pruning.
- Added retention validation for adapter/run/candidate identity, detached state, clean status, exact direct-child topology, and compare-and-swap branch advance.
- Added refusal for branch divergence, checked-out retained refs, stale values, foreign values, existing targets, symlinked parents, dirty candidates, attached candidates, and multi-commit candidates.
- Added integration proof that caller branch, HEAD, index, and porcelain status remain unchanged through prepare and retain.

## Files Created/Modified

- `crates/autoresearch-core/src/workspace.rs` - Pure workspace values and inward-owned lifecycle port.
- `crates/autoresearch-core/tests/workspace.rs` - Domain invariant tests.
- `crates/autoresearch-git/src/error.rs` - Shared typed infrastructure failures.
- `crates/autoresearch-git/src/workspace.rs` - Lock-bound run branch and candidate worktree adapter.
- `crates/autoresearch-git/tests/workspace.rs` - Temporary-repository lifecycle integration tests.
- `crates/autoresearch-git/src/repository.rs` and `src/lock.rs` - Shared error extraction, internal command reuse, and canonical run-ID validation.
- `.planning/ROADMAP.md` and `.planning/phases/02-git-loop/02-02-PLAN.md` - Slice definition and progress.

## Decisions Made

- Run IDs now permit 1-128 ASCII letters, digits, dashes, or underscores. Dots were removed because Git ref rules and path semantics differ; one strict shared format avoids adapter drift.
- Retained run branch is never checked out. Adapter moves it only through `git update-ref` with expected-old commit.
- Candidate worktree stays detached and Git-locked so branch ownership remains unambiguous and pruning cannot erase unresolved evidence.
- Adapter deliberately does not stage or commit changes yet. Plan 02-03 must validate changed paths and symlink containment before creating candidate commit.
- Retention accepts exactly one clean direct-child commit. Multiple commits and merges fail closed, keeping one candidate equal to one comparable revision.
- Worktree deletion remains deferred to journal-driven recovery in Plan 02-04.

## Issues Encountered

- `git show-ref --hash` parsing differed from assumed missing-ref behavior. Exact optional resolution now uses `git rev-parse --verify --quiet <ref>^{commit}` and has regression coverage.
- Prime graph registration was attempted but local AllSource instance was read-only because another process owned writer data directory. Planning files remain canonical.
- Machine-wide `sccache` remained unavailable in sandbox. Verification used empty `CARGO_BUILD_RUSTC_WRAPPER` without altering global or repository config.

## Verification

- `cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` - passed.
- `cargo test --workspace --all-features --offline` - 62 passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --offline` - passed.
- Core dependency scan for Git, process, filesystem I/O, and infrastructure imports - clean.

## Next Step

Phase 2 Plan 3: validate mutable/protected paths, reject nested repositories and symlink escapes, then stage and create one contained candidate commit.
