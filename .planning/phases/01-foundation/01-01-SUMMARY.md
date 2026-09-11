# Phase 1 Plan 1: Workspace and Decision Core Summary

**Strict Rust workspace now makes candidate promotion deterministic and keeps non-objective evidence outside selection.**

## Accomplishments

- Added edition-2024 Cargo workspace with inherited lint policy, pinned toolchain, lockfile, and CI.
- Added validated finite measurements, typed hard gates, objective directions, and candidate complexity.
- Implemented fail-closed lexicographic selection with explicit machine-readable decisions.
- Covered hard-gate failure, maximize/minimize improvement, regression, complexity ties, malformed snapshots, and invalid baselines with 10 unit tests.

## Files Created/Modified

- `Cargo.toml` - Workspace members, dependencies, metadata, and quality policy.
- `rust-toolchain.toml` - Pinned Rust, Clippy, and rustfmt.
- `crates/autoresearch-core/` - Measurement and decision domain model.
- `.github/workflows/ci.yml` - Format, Clippy, test, and rustdoc gates.
- `.gitignore` - Rust output and private run-state exclusions.
- `.planning/` - Brief, roadmap, executable plan, and outcome record.

## Decisions Made

- Hard gates use boolean outcomes; numeric roles use a separate enum, preventing invalid gate/value combinations.
- Numeric values reject NaN and infinity at construction.
- Exact primary equality resolves by changed lines, dependency delta, then runtime; an exact tie discards.
- Diagnostic and market-evidence measurements remain serialisable but cannot influence selection.

## Issues Encountered

- Local `sccache` cannot run inside filesystem sandbox. Verification used direct `rustc` via empty `RUSTC_WRAPPER`; CI does not use local wrapper.
- Dependency fetch required network access once. Locked versions now recorded in `Cargo.lock`.

## Verification

- `cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` - passed.
- `cargo test --workspace --all-features` - 10 passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps` - passed.

## Next Step

Ready for `01-02-PLAN.md`: manifest schema, validation, and canonical frozen identity.
