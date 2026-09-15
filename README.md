# autoresearch-rs

[![CI](https://github.com/wolven-tech/autoresearch-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/wolven-tech/autoresearch-rs/actions/workflows/ci.yml?query=branch%3Amain)
[![License: MIT](https://img.shields.io/github/license/wolven-tech/autoresearch-rs)](LICENSE)
[![Rust toolchain](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fwolven-tech%2Fautoresearch-rs%2Fmain%2Frust-toolchain.toml&query=%24.toolchain.channel&label=rust%20toolchain&color=orange)](rust-toolchain.toml)

A candidate added a paragraph to a product page, raised its score from 2 to 3, and dropped the Start link. autoresearch-rs discarded it without comparing the score, because keeping that link was a hard gate frozen before the run started.

That is the idea. Freeze what must not break, then let one change at a time compete on a number you own. A higher score never rescues a broken gate, and every keep and discard is journaled with the reason.

![Two metal experiment modules approach a fixed glass evaluation gate; one rests in an illuminated keep cradle, the other sits in a shadowed discard tray](assets/social/autoresearch-rs-experiment-gate-2026-09-14.png)

The image is concept art from the launch, not a benchmark or a screenshot. The discard it illustrates is a real test in [the CLI suite](apps/autoresearch-cli/tests/cli.rs).

Start with [your first loop](docs/tutorials/first-loop.md), a lesson on an example that is already set up. The rest of [the documentation](docs/README.md) is split by what you need: how-to guides for a task, reference for the CLI, manifest and protocol, and explanation for why a run decides the way it does, what [six earlier loops](docs/explanation/case-studies.md) measured, and [what a keep does not prove](docs/explanation/limits.md).

autoresearch-rs is a Rust fork of Andrej Karpathy's [autoresearch](https://github.com/karpathy/autoresearch), maintained by Decebal Dobrica at Wolven Tech, with the upstream Python files kept unmodified in [reference/python](reference/python/README.md). It is MIT licensed apart from those files; see [LICENSE](LICENSE) and [NOTICE](NOTICE).
