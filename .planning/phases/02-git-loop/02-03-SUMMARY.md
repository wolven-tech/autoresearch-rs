# Phase 2 Plan 3: Contained Candidate Commit Summary

**Only frozen-scope regular-file mutations can now become one comparable, hook-free candidate commit.**

## Accomplishments

- Promoted repository-relative path validation into core so manifest parsing and commit enforcement use one canonical model.
- Added normalized mutable roots with protected descendant carve-outs and protected-first containment decisions.
- Added core `CandidateCommit` evidence and `CandidateCommitter` port while preserving `autoresearch_config::RepoPath` and `Scope` imports.
- Added NUL-delimited staged, unstaged, untracked, committed, and ignored path inspection.
- Added rejection for no-op candidates, ignored state, non-UTF-8 names, symlinks, Git symlink/gitlink modes, nested repositories, protected paths, and paths outside mutable roots.
- Added linked-worktree identity proof across canonical root, common Git directory, administrative directory, and backlink.
- Added hook-free candidate creation through `write-tree`, `commit-tree`, and compare-and-swap detached `HEAD` update.
- Added adversarial integration proof that allowed create/edit/delete changes form exactly one direct-child commit while caller checkout and retained run ref remain untouched.

## Files Created/Modified

- `crates/autoresearch-core/src/containment.rs` - Pure path, boundary, commit-evidence, and committer-port contract.
- `crates/autoresearch-core/src/lib.rs` and `tests/containment.rs` - Stable re-exports and domain invariant coverage.
- `crates/autoresearch-config/src/manifest.rs` and `src/lib.rs` - Core-backed scope validation with stable public re-exports.
- `crates/autoresearch-git/src/workspace.rs` - Shared candidate identity precondition and contained-commit module routing.
- `crates/autoresearch-git/src/workspace/commit.rs` - Diff collection, containment enforcement, staging, and plumbing commit creation.
- `crates/autoresearch-git/src/workspace/identity.rs` - Linked-worktree administrative identity checks.
- `crates/autoresearch-git/src/error.rs` - Typed containment, path-shape, ignored-state, identity, and race failures.
- `crates/autoresearch-git/tests/containment.rs` - Temporary-repository success and hostile-state fixtures.
- `.planning/ROADMAP.md` and `.planning/phases/02-git-loop/02-03-PLAN.md` - Slice definition and progress.

## Decisions Made

- Core owns `RepoPath`, `MutationBoundary`, candidate commit evidence, and port; config remains a parser/normalizer and Git remains infrastructure.
- Protected roots are evaluated before mutable roots, permitting broad mutable roots with narrow immutable carve-outs.
- Candidate filenames must be portable UTF-8 without ASCII control characters; ambiguous diagnostics and platform-dependent names fail closed.
- Ignored files fail candidate creation because evaluator-visible state absent from commit would not be reproducible.
- Mutation agents may stage changes, but adapter restages complete visible worktree only after validating union of staged, unstaged, and untracked paths.
- Candidate commits use fixed runner-generated messages and Git plumbing. Repository hooks and shell evaluation never run.
- Commit creation advances only detached candidate `HEAD`; retained run ref moves later through explicit selection and retention.
- Worktree identity logic lives in a focused module so every source module stays below 500 lines.

## Issues Encountered

- Sandbox refused creation of a non-UTF-8 filename with `Operation not permitted`. Integration coverage stages an invalid byte path directly in temporary Git index, exercising same raw Git output failure without weakening test.
- Initial post-stage race check compared worktree against `HEAD`, which includes staged paths. It now compares worktree against index and separately inspects staged diff.
- Prime graph registration remained unavailable because another process owns AllSource writer data directory; planning files remain canonical.
- Two global rule references linked by installed skills were absent. Installed skill checklists and repository gates were applied directly.

## Verification

- `CARGO_BUILD_RUSTC_WRAPPER= cargo fmt --all --check` - passed.
- `cargo sort --workspace --check` - passed.
- `CARGO_BUILD_RUSTC_WRAPPER= cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` - passed.
- `CARGO_BUILD_RUSTC_WRAPPER= cargo test --workspace --all-features --offline` - 72 passed.
- `CARGO_BUILD_RUSTC_WRAPPER= RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --offline` - passed.
- Core dependency scan for process, filesystem, Git, config, and infrastructure imports - clean.
- Git commit-adapter security grep for shell construction, `git commit`, hook invocation, and deletion operations - clean.
- Candidate adapter modules are 479, 398, and 134 lines - below 500-line target.

## Security Review

No findings. Candidate commit path uses literal Git argument arrays, NUL-delimited path records, canonical worktree identity, protected-first scope checks, fixed commit messages, and compare-and-swap ref updates. CRITICAL/HIGH/MEDIUM all clear for reviewed command-injection and path-containment surface.

## Next Step

Phase 2 Plan 4: replay journal state to recover incomplete Git transitions and remove only proven run-owned candidate worktrees.
