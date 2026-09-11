# Phase 2 Plan 1: Repository Validation and Lock Summary

**Experiments now fail before mutation when repository identity, cleanliness, run-state isolation, or exclusive ownership is unsafe.**

## Accomplishments

- Added core-owned full commit IDs, validated repository snapshots, and read-only repository inspection port.
- Added `autoresearch-git` infrastructure crate using direct Git argument arrays without shell evaluation.
- Added typed rejection for missing/non-directory targets, non-worktrees, bare repositories, blank or unresolved refs, dirty tracked/untracked state, tracked or unignored run state, and invalid Git identities.
- Added advisory `.autoresearch/run.lock` ownership with bounded JSON metadata and automatic kernel release on guard drop or process exit.
- Added symlink refusal and under-lock repository revalidation so stale snapshots cannot acquire ownership.
- Added temporary-Git integration coverage for clean inspection, failure classes, concurrent ownership, reacquisition, independent repositories, stale HEAD, and symlink protection.

## Files Created/Modified

- `crates/autoresearch-core/src/repository.rs` - Pure repository identity values and inspection port.
- `crates/autoresearch-git/` - Git inspection adapter, advisory lock, and integration tests.
- `Cargo.toml` and `Cargo.lock` - Git crate workspace membership and existing serialization dependencies.
- `.planning/ROADMAP.md` - Four-plan Git Loop breakdown and progress.
- `.planning/phases/02-git-loop/02-01-PLAN.md` - Executed atomic slice.

## Decisions Made

- Core owns repository values and port; infrastructure owns process and filesystem I/O.
- Git uses installed CLI directly rather than `git2`, preserving native worktree behavior and avoiding shell parsing.
- Repository must be entirely clean, not only clean on protected paths, before run ownership begins.
- Lock file remains as last-owner evidence; operating-system advisory ownership, not file existence, determines liveness.
- Lock acquisition revalidates root, base, HEAD, and cleanliness under lock to reject stale snapshots.
- Run IDs are limited to 256 safe ASCII filename/metadata characters.

## Issues Encountered

- Machine-wide Cargo config routes Rust through `sccache`, which sandbox denied with `Operation not permitted`. Quality commands used empty `CARGO_BUILD_RUSTC_WRAPPER` without changing repository or global config.
- Rust 1.97 `File::try_lock` returns typed `TryLockError`; implementation handles `WouldBlock` separately from operating-system failures.
- macOS temporary directory lexical path differs from canonical `/private/var`; tests now compare canonical snapshot paths.

## Verification

- `cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` - passed.
- `cargo test --workspace --all-features --offline` - 47 passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --offline` - passed.
- Core dependency scan for `autoresearch-git` and `git2` - clean.

## Next Step

Phase 2 Plan 2: create one retained run branch and isolated candidate worktree without moving or editing caller checkout.
