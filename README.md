# autoresearch-rs

Rust experiment engine for frozen, falsifiable product and training research.
This fork retains upstream Python source and history in
[`reference/python`](reference/python/README.md). Provenance and invocation:
[`reference/python/MANIFEST.md`](reference/python/MANIFEST.md).

## First usable Rust loop

Workspace ships frozen contracts, isolated Git candidate worktrees, bounded
serial manual or explicitly allowlisted command mutations, exact-commit
evaluation, journal recovery, independent kept-commit verification, and
versioned JSON/HTML reports with bounded redacted local export. Product Web
adapters cover local Chromium routes, bounded accessibility evidence, SEO/GEO
diagnostics, frozen Lighthouse report import, and permission-gated read-only
production probes. This is not autonomous orchestration or proof of customer
demand, rankings, AI citations, WCAG AA conformance, or product-bet promotion.

```bash
cargo run -p autoresearch-cli -- --help
cargo run -p autoresearch-cli -- --repository /path/to/product init
# Review and commit autoresearch.toml, program.md, and .gitignore before baseline.
cargo run -p autoresearch-cli -- --repository /path/to/product doctor
cargo run -p autoresearch-cli -- --repository /path/to/product baseline
cargo run -p autoresearch-cli -- --repository /path/to/product run --run-id RUN_ID --mode manual
# Edit only declared mutable paths in reported isolated worktree.
cargo run -p autoresearch-cli -- --repository /path/to/product run --run-id RUN_ID --mode manual --hypothesis "one falsifiable change"
cargo run -p autoresearch-cli -- --repository /path/to/product status --run-id RUN_ID
cargo run -p autoresearch-cli -- --repository /path/to/product resume --run-id RUN_ID
cargo run -p autoresearch-cli -- --repository /path/to/product verify --run-id RUN_ID
cargo run -p autoresearch-cli -- --repository /path/to/product report --run-id RUN_ID --html
cargo run -p autoresearch-cli -- --repository /path/to/product export --run-id RUN_ID --export-root /absolute/existing/review-dir
```

`init` preserves existing files. `doctor` does not execute agents/evaluators.
`baseline` freezes clean, tracked control inputs and evaluates exact commit.
`run` never edits caller checkout. Manual mode submits changes from reported
worktree; command mode needs matching frozen executable plus explicit
`--allow-executable`. `resume` follows journal state and refuses ambiguous Git
effects. `verify` reruns frozen evaluators without reselecting. `report` is
read-only; `export` creates new bundle outside product checkout and copies only
explicitly selected, report-declared artifacts. Neither command deploys,
publishes, contacts customers, collects payment, or moves product gates.

Disposable product-page CLI fixture covers baseline, kept improvement,
higher-scoring failed-CTA-gate discard after simulated post-commit crash,
recovery, verify, report, and export. Its objective counts content items, not
live page quality. Separate [offline Product Web pack](examples/product-web/README.md)
exercises Chromium and source-backed adapters; Lighthouse samples there are
synthetic import fixtures.

Details: [architecture](docs/plans/2026-09-11-autoresearch-rust-platform-design.md),
[evaluator protocol](docs/evaluator-protocol.md),
[evaluator examples](docs/evaluator-examples.md),
[evidence board and export](docs/evidence-board.md).

## Candle and portfolio handoff

[Tiny Candle candidate example](examples/nanochat-candle/README.md) runs a
bounded CPU/f32 experiment in a disposable Git repository. It is distinct
from the [first usable product loop](#first-usable-rust-loop). The
[training substrate](crates/autoresearch-training-candle/README.md) describes
fixed-budget SGD, exact fixture, checkpoints, and evaluator adapter.
[Cross-language parity status](docs/training-parity.md) and its
[frozen report](fixtures/tiny/parity-report.json) record exact matches,
tolerances, and known optimizer discrepancy. This is not upstream nanochat,
CUDA, H100, or full-model training parity.

[Portfolio templates](examples/portfolio/README.md) cover UI, copy,
performance, SEO, GEO, calculator, and mobile local diagnostics. Each pack
needs product-specific paths and pinned evaluator binaries before use.
Placeholders intentionally fail. Manifests omit external authority by
default; they do not provide OS-level network isolation for arbitrary child
processes. Lab metrics, traffic, and praise do not move product bet gates.

## Clean-checkout verification

With Rust toolchain and offline Cargo dependencies available, run from a
fresh checkout at repository root:

```text
cargo fmt --all --check
RUSTC_WRAPPER= cargo test -p autoresearch-config --offline --test tiny_fixture --test portfolio_packs
RUSTC_WRAPPER= cargo test -p autoresearch-training-candle --offline --test data --test model --test evaluator
RUSTC_WRAPPER= cargo test -p autoresearch-training-candle --offline --test evaluator disposable_candidate_produces_exact_decision_journal_and_report -- --exact
RUSTC_WRAPPER= cargo test --workspace --all-features --offline -j 2 -- --test-threads=1
RUSTC_WRAPPER= cargo clippy --workspace --all-targets --all-features --offline -j 2 -- -D warnings
RUSTC_WRAPPER= RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --offline --no-deps -j 2
```

Browser tests require installed Chromium and loopback bind permission. The
Python stdlib comparison is separate **opt-in** verification:

```text
RUSTC_WRAPPER= cargo test -p autoresearch-training-candle --offline --test parity -- --ignored --nocapture
```

CUDA/Metal/half-precision training is unavailable in this crate, not a
skipped passing check. No optional hardware result is claimed here.
