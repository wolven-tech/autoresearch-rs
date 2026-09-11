# Phase 1 Plan 4: Foundation CLI Summary

**Repositories can now create, diagnose, and freeze experiment contracts without mutation, command execution, or fabricated evidence.**

## Accomplishments

- Added `autoresearch` CLI with `init`, `doctor`, `baseline`, text output, JSON output, and stable exit classes.
- Added create-new contract templates; repeated init preserves existing bytes.
- Added read-only checks for Git worktree, schema, program, ignored run state, and configured executables.
- Added baseline freeze transaction for exact HEAD, validated config, program, optional `docs/BET.md`, identity JSON, immutable copies, and sequence-zero JSONL journal.
- Added unit and process-level Git fixture tests; workspace now has 33 passing tests.
- Added Rust foundation quick start while preserving upstream Python documentation and behavior.

## Files Created/Modified

- `apps/autoresearch-cli/` - Binary, command implementation, templates, baseline persistence, and tests.
- `Cargo.toml` and `Cargo.lock` - CLI workspace member and Clap dependency.
- `README.md` - Honest foundation status and usage.
- `.planning/phases/01-foundation/01-04-PLAN.md` - Executable slice definition.
- `.planning/ROADMAP.md` - Phase 1 completion.

## Decisions Made

- `doctor` checks executable presence but never invokes configured agent or evaluator.
- `baseline` freezes inputs and opens journal only; output says evaluator evidence is pending.
- Exit code 3 represents config/identity rejection, 4 environment/state rejection, and 5 internal I/O/serialization failure. Clap retains code 2 for usage.
- Existing `docs/BET.md` is automatically included as original product gate and must be tracked and clean.
- `.autoresearch/` must be ignored before baseline can write evidence.

## Issues Encountered

- Host Git globally enables commit signing. Integration fixtures explicitly disable signing locally.
- Initial cleanliness check could miss ignored untracked contracts. Added `git ls-files --error-unmatch` before status check.
- Run ID collision suffix initially diverged from journal ID. Directory allocator now returns exact selected ID.

## Verification

- `cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` - passed.
- `cargo test --workspace --all-features --offline` - 33 passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --offline` - passed.
- `cargo run -q -p autoresearch-cli --offline -- --help` - lists only `init`, `doctor`, and `baseline`.

## Next Step

Phase 1 complete. Ready for Phase 2 Git Loop: validated isolated worktrees, containment, run branches, locks, and recovery-aware cleanup.
