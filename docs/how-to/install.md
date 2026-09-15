# How To: Install autoresearch-rs

## Prerequisites

- Rust 1.97.1 (pinned in [rust-toolchain.toml](../../rust-toolchain.toml))
- `git` on your PATH with an author identity configured (`git config user.name` and `git config user.email`)

Check your Rust version:

```bash
rustc --version
rustup show
```

If you have an older Rust version, upgrade:

```bash
rustup update
```

## Build from source

Clone the repository:

```bash
git clone https://github.com/wolven-tech/autoresearch-rs
cd autoresearch-rs
```

Build the CLI:

```bash
cargo build --release -p autoresearch-cli
```

The binary is at `target/release/autoresearch`.

Add it to your PATH:

```bash
export PATH="$PWD/target/release:$PATH"
```

Or make it permanent by adding that line to your shell profile (`.bashrc`, `.zshrc`, etc.).

Verify the installation:

```bash
autoresearch --version
```

## Install as a command

Install the binary into your cargo bin directory:

```bash
cargo install --path apps/autoresearch-cli
```

This installs the binary to `~/.cargo/bin/autoresearch`. If that directory is on your PATH, you can run:

```bash
autoresearch --version
```

from anywhere.


## Verify the installation

Test that the CLI is working:

```bash
autoresearch --version
autoresearch --help
```

Test that you can initialize an experiment:

```bash
mkdir -p /tmp/autoresearch-test
cd /tmp/autoresearch-test
git init
git config user.email "test@example.com"
git config user.name "Test User"
autoresearch --repository . init
```

You should see `autoresearch.toml` and `program.md` created.

Clean up:

```bash
cd /
rm -rf /tmp/autoresearch-test
```

## Try a complete workflow

Once installed, you can try the basic commands:

```bash
autoresearch doctor
```

This checks your configuration and environment. For a complete experiment, see the [first-loop tutorial](../tutorials/first-loop.md).

Examples of commands you'll use in a real experiment:

```bash
autoresearch baseline
autoresearch run --run-id run-example --hypothesis "my change"
autoresearch verify --run-id run-example
autoresearch report --run-id run-example
```

## Limitations

Autoresearch-rs does not provide:
- Autonomous orchestration or scheduling beyond one bounded step per CLI call.
- Evidence of customer demand, market rankings, or AI citations.
- WCAG AA conformance or a complete accessibility audit (keyboard traversal and contrast sampling cover a subset of controls).
- SEO or GEO verification (local lint only; results are not verified against live services).
- Network or filesystem isolation; operators must provide a sandbox before running untrusted code.
- Process cleanup for arbitrary descendant processes.
- Training parity with upstream: CUDA, Metal, f16 and bf16 are unavailable; values are not comparable with upstream BPE scores; optimizer differs from upstream.
- External side effects or outreach: the system does not make network requests outside the experiment, does not deploy changes, and does not contact external services.

## Troubleshooting

If "rustc not found" appears, you need to install Rust. Follow the [official Rust installation instructions](https://rustup.rs).

If "cargo build fails with 'unsupported rustc version'" appears, your Rust version is too old. Run `rustup update` to upgrade to the latest stable.

If "autoresearch command not found" appears, the binary is not on your PATH. Either add the build directory to PATH with `export PATH="$PWD/target/release:$PATH"`, use the full path `/path/to/autoresearch-rs/target/release/autoresearch`, or install with `cargo install --path apps/autoresearch-cli` and ensure `~/.cargo/bin` is on your PATH.

If "permission denied" appears when running autoresearch, make sure the binary is executable:

```bash
chmod +x /path/to/autoresearch
```

If "evaluator not found" appears when running baseline, the manifest declares an evaluator path that does not exist. Write your evaluator first (see [write-an-evaluator.md](write-an-evaluator.md)), then update the manifest to point at it.

## What to do next

Follow the [first-loop tutorial](../tutorials/first-loop.md) to run a complete experiment. Read the [CLI reference](../reference/cli.md) to understand all the commands. Follow [write-an-evaluator.md](write-an-evaluator.md) to learn how to build your evaluator.
