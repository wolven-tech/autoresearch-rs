# Phase 2 Plan 4: Journal-Driven Git Recovery Summary

**Replayed journal state now converges interrupted candidate Git effects and removes only exact, clean, run-owned worktrees.**

## Accomplishments

- Bound each durable candidate decision to exact evaluated candidate commit and required keep finalization to match it.
- Added pure recovery requests, outcomes, candidate-stage classification, and inward-owned recovery port.
- Added idempotent recovery for missing/unlocked prepared worktrees, clean parent candidates, and committed direct-child candidates.
- Added idempotent keep recovery before or after retained-ref advance and discard recovery before or after worktree removal.
- Added NUL-delimited Git worktree registry parsing plus exact filesystem, repository, detached-HEAD, lock-owner, and commit-topology proof.
- Added safe cleanup through `git worktree unlock` and `git worktree remove` without recursive filesystem deletion, force, pruning, shell evaluation, or caller-checkout mutation.
- Added refusal and preservation for dirty or ignored state, foreign lock reasons, unregistered path collisions, wrong candidate commits, nonlinear history, and conflicting retained refs.

## Files Created/Modified

- `crates/autoresearch-core/src/journal.rs` - Candidate decisions now bind exact candidate commits and finalization verifies them.
- `crates/autoresearch-core/src/recovery.rs` - Pure recovery request/result values and application-facing port.
- `crates/autoresearch-core/src/lib.rs` - Stable recovery API exports.
- `crates/autoresearch-core/tests/recovery.rs` - Journal-to-candidate derivation and invalid-identity proof.
- `crates/autoresearch-git/src/workspace.rs` - Shared ownership checks and ignored-state cleanliness enforcement.
- `crates/autoresearch-git/src/workspace/recovery.rs` - Lock-bound reconciliation and exact cleanup adapter.
- `crates/autoresearch-git/src/error.rs` - Typed recovery, registration, and lock conflicts.
- `crates/autoresearch-git/tests/recovery.rs` - Crash-boundary, idempotency, preservation, and caller-isolation fixtures.
- `docs/plans/2026-09-11-autoresearch-rust-platform-design.md` - Exact commit binding recorded in recovery contract.
- `.planning/ROADMAP.md` and `.planning/phases/02-git-loop/02-04-PLAN.md` - Completed Phase 2 slice and tracking.

## Decisions Made

- Journal decision record, not commit message or inferred topology, is authoritative candidate commit identity.
- Recovery returns `None` for baseline, next-candidate, and stopped states; only replayed evaluate/finalize actions may touch candidate Git state.
- Clean parent candidate means mutation pending; clean direct child means evaluator work may resume. Dirty state is ambiguous and remains untouched.
- Keep advances retained ref before cleanup. Recovery recognizes either parent or exact journal-bound candidate commit and rejects every third state.
- Discard never moves retained ref. Missing worktree after recorded discard is idempotent completion; registration/path disagreement fails closed.
- Unlocked exact worktree is accepted only as interrupted preparation/finalization; unexpected lock reason is foreign ownership.
- Git owns worktree deletion. Adapter never directly recursively deletes candidate paths and never uses force on evidence-bearing state.

## Issues Encountered

- Initial recovery draft could only identify an already-cleaned kept commit by generated subject. Security review rejected that inference; journal schema now carries exact candidate commit identity.
- Machine-wide Cargo wrapper remains unavailable inside sandbox. Verification used empty `CARGO_BUILD_RUSTC_WRAPPER` without changing global or repository config.
- Prime plan registration remained unavailable because another process owns AllSource writer directory; planning files remain canonical.

## Verification

- `CARGO_BUILD_RUSTC_WRAPPER= cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `CARGO_BUILD_RUSTC_WRAPPER= cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` - passed.
- `CARGO_BUILD_RUSTC_WRAPPER= cargo test --workspace --all-features --offline` - 80 passed.
- `CARGO_BUILD_RUSTC_WRAPPER= RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --offline` - passed.
- Recovery security scan for shell execution, recursive deletion, force removal, pruning, and broad paths - clean.
- Core recovery dependency scan for Git, process, filesystem I/O, config, and infrastructure imports - clean.
- New adapter/test modules and shared workspace adapter remain at or below 500-line target.

## Security Review

No findings after exact-commit binding correction. Cleanup requires live matching lock, deterministic candidate path, matching Git registration, canonical real directory, linked-worktree backlink, shared common directory, detached HEAD, expected lock owner or interrupted unlock, clean including ignored state, and exact journal-bound topology. Any mismatch preserves evidence.

## Next Step

Phase 2 complete. Ready to plan Phase 3 evaluator SDK contracts and adapters.
