<div align="center">

# autoresearch-rs

**Automated hypothesis testing with frozen gates and competitive scoring.**<br>
Freeze hard requirements, then let changes compete on metrics. Every keep or discard is journaled with the reason.

[![crates.io](https://img.shields.io/crates/v/autoresearch?logo=rust&logoColor=white&color=e43716)](https://crates.io/crates/autoresearch)
[![CI](https://img.shields.io/github/actions/workflow/status/wolven-tech/autoresearch-rs/ci.yml?branch=main&logo=githubactions&logoColor=white&label=CI)](https://github.com/wolven-tech/autoresearch-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT%20%2B%20upstream-blue)](LICENSE)
[![Upstream: karpathy/autoresearch](https://img.shields.io/badge/Upstream-karpathy%2Fautoresearch-181717?logo=github&logoColor=white)](https://github.com/karpathy/autoresearch)
[![Rust 1.84+](https://img.shields.io/badge/Rust-1.84+-e74c3c?logo=rust&logoColor=white)](#install)

[![Rust, built with Cargo](https://skillicons.dev/icons?i=rust,cargo)](https://skillicons.dev)

[Install](#install) · [First loop](docs/tutorials/first-loop.md) · [Docs](docs/README.md) · [Case studies](docs/explanation/case-studies.md) · [Upstream](https://github.com/karpathy/autoresearch)

</div>

---

**A fork of Andrej Karpathy's [autoresearch](https://github.com/karpathy/autoresearch), rewritten in Rust and maintained by Wolven Tech.**

The upstream Python files are kept unmodified in [`reference/python`](reference/python/README.md) for reference. This Rust implementation retains the core design: define hard gates that cannot be overridden by score, then let one candidate at a time compete on metrics you control. Every decision is journaled.

Real case: a candidate added a paragraph that raised a product page's score but broke a link that was a hard requirement. The gate rejected it without comparing scores — the broken link was frozen before the run started.

| Capability | Details |
|---|---|
| **Hard gates** | Frozen requirements; no score rescues a failed gate |
| **Competitive scoring** | Change with higher score is kept; reason is journaled |
| **Journaling** | Every keep, discard, and gate failure logged with reason |
| **Configuration** | YAML manifest specifies gates, metrics, and candidate sources |
| **Protocol** | Extensible evaluation framework for custom gates and metrics |

## Install

```sh
cargo install autoresearch
```

Or clone and build:

```sh
git clone https://github.com/wolven-tech/autoresearch-rs
cd autoresearch-rs
cargo build --release
```

## What it does

| Capability | Where |
|---|---|
| **Gate evaluation** | Candidate must pass all hard gates or is rejected immediately |
| **Metric scoring** | Gate-passing candidates compete on configured metrics (higher is better) |
| **Keep decision** | Candidate with highest score is kept; previous best is discarded |
| **Journaling** | Every evaluation logged with candidate, gate status, scores, and reason |
| **Resumption** | Run can resume from journal; no duplicate evaluations |

## First steps

See [your first loop](docs/tutorials/first-loop.md) for a step-by-step lesson using an example that is already set up.

Full documentation is split by what you need:

- **How-to guides** — performing a task
- **Reference** — CLI, manifest format, protocol
- **Explanation** — why a run decides the way it does, six earlier case studies, and what a keep does not prove

**[Read the docs →](docs/README.md)**

## Case studies

[Six earlier loops](docs/explanation/case-studies.md) show what autoresearch-rs measured in real product work and research scenarios.

## Limits

- Candidate sources are pluggable; built-in source is a directory of variants.
- Gates must be deterministic; non-deterministic gates produce inconsistent results.
- Score is a single number; tie-breaking is by timestamp (first to score wins).

See [what a keep does not prove](docs/explanation/limits.md) for the full analysis.

## Upstream

This is a Rust fork of Andrej Karpathy's [autoresearch](https://github.com/karpathy/autoresearch). The original Python implementation is preserved in [`reference/python/`](reference/python/README.md).

**MIT License** applies to all Rust code. The Python reference files are part of the upstream project and carry their own license. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

## License

MIT (Rust implementation)

Upstream files ([`reference/python/`](reference/python/README.md)) are from [github.com/karpathy/autoresearch](https://github.com/karpathy/autoresearch).
