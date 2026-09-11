# Phase 1 Plan 2: Frozen Manifest Summary

**Versioned TOML now fails closed before mutation and hashes every frozen comparison input into relocation-independent evidence.**

## Accomplishments

- Added validated schema for bounded experiments, mutation scope, agent command, evaluators, output definitions, and authority ceilings.
- Normalized repository-relative paths and automatically protected `autoresearch.toml` and `program.md`.
- Enforced one globally unique primary objective with frozen direction.
- Added SHA-256 component and aggregate identities for manifest, program, optional product gate, and named fixtures.
- Added 10 config/identity tests; workspace now has 20 passing unit tests.

## Files Created/Modified

- `Cargo.toml` and `Cargo.lock` - Config, TOML, JSON, and SHA-256 dependencies.
- `crates/autoresearch-config/` - Manifest and identity implementation.
- `.planning/phases/01-foundation/01-02-PLAN.md` - Executable slice definition.
- `.planning/ROADMAP.md` - Foundation progress.

## Decisions Made

- Parsed raw TOML never escapes config crate; callers receive only `ValidatedManifest`.
- Command specs use executable plus literal argument arrays, not shell strings.
- External permissions use ordered capability set. Manifest declares ceiling; it never grants authority by itself.
- Mutable path ordering is semantically irrelevant and normalized. Evaluator and argument order remain identity-significant.
- Absolute paths and filesystem metadata remain outside identity, allowing safe worktree relocation.

## Issues Encountered

- Cargo dependency download required network access once. Lockfile now enables offline verification.
- Clippy rejected boolean-heavy authority model. Replaced it with typed `ExternalCapability` set.

## Verification

- `cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` - passed.
- `cargo test --workspace --all-features --offline` - 20 passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --offline` - passed.

## Next Step

Ready for `01-03-PLAN.md`: append-only journal and CLI init/doctor/baseline shell.
