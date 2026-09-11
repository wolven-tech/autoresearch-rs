//! Initial repository contract templates.

pub(crate) const MANIFEST: &str = r#"# Autoresearch experiment contract.
# Commit this file before `autoresearch baseline`. It becomes frozen per run.
schema_version = 1

[experiment]
name = "small repository improvement"

[experiment.objective]
name = "changed_lines"
direction = "minimize"

[experiment.budget]
max_candidates = 8
max_failures = 2
wall_clock_seconds = 1800

[scope]
mutable_paths = ["src"]
protected_paths = []

# Replace command with preferred mutation agent. Arguments are literal; no shell.
[agent]
program = "git"
args = ["status", "--short"]
timeout_seconds = 600

# Foundation sample proves schema and command availability only. Evaluator SDK
# will enforce JSONL output before experiments can capture baseline evidence.
[[evaluators]]
id = "diff"
hard_gates = ["repository_valid"]

[evaluators.command]
program = "git"
args = ["diff", "--numstat"]
timeout_seconds = 60

[[evaluators.metrics]]
name = "changed_lines"
kind = "objective"
direction = "minimize"

[authority]
allow = []
"#;

pub(crate) const PROGRAM: &str = r"# Research program

## Objective

Improve one declared repository surface against frozen evaluators.

## Rules

- Change only paths listed in `autoresearch.toml`.
- Do not edit manifest, this program, evaluators, fixtures, or product gate.
- Form one falsifiable hypothesis per candidate.
- Do not deploy, purchase, contact people, change permissions, or write to production.
- Treat traffic, clicks, impressions, praise, and synthetic events as non-promotional evidence.
";
