# autoresearch-rs documentation

Four kinds of page, split by what you need at the moment you arrive. If you are not sure, run the tutorial first; it is the only page that assumes you know nothing.

## Tutorials

A lesson to work through once, start to finish, on an example that is already set up. The contract, the page it scores and the evaluator are all provided, so the only thing you supply is the reading. Everything it prints is shown in the page, which makes it the fastest way to find out whether the loop suits the way you work.

- [Your first loop](tutorials/first-loop.md): freeze a contract, evaluate a baseline, submit one candidate, and read the decision.

## How-to guides

Task-shaped pages for when you know what you want and need the steps. They assume you have run a loop before, so they skip the explanation and name the commands, the files they touch and the state they expect.

| Guide | Use it when |
| --- | --- |
| [Install the CLI](how-to/install.md) | you need the binary and the toolchain it expects |
| [Write an evaluator](how-to/write-an-evaluator.md) | you need the program that produces your number |
| [Recover a run](how-to/recover-a-run.md) | a crash, a dirty tree or a moved HEAD stopped a run |
| [Take a kept result](how-to/take-a-kept-result.md) | a candidate was kept and you want it on your branch |
| [Hand a loop to an agent](how-to/hand-a-loop-to-an-agent.md) | Claude Code, the skill, or command mode will make the edits |

## Reference

Facts to look up, kept in the shape of the thing they describe. Nothing here tells you what to do with a fact; when a reference page and the code disagree, the code is right and the page is a defect.

| Page | Covers |
| --- | --- |
| [CLI](reference/cli.md) | subcommands, flags, output fields, exit codes |
| [Manifest](reference/manifest.md) | every `autoresearch.toml` table and its validation rules |
| [Evaluator protocol](reference/evaluator-protocol.md) | the JSONL request and response, field by field |
| [Decision policy](reference/decision-policy.md) | the order, the tie-break fields, the reason codes |
| [Run state](reference/run-state.md) | what lives under `.autoresearch/`, and the journal |
| [Repository effects](reference/repository-effects.md) | refs, worktrees, commits and preconditions |
| [Report and export](reference/report-and-export.md) | what a bundle contains and what is redacted |
| [Evaluator examples](reference/evaluator-examples.md) | worked evaluators, with [training parity](reference/training-parity.md) for the Candle lane |

## Explanation

Background for decisions you are weighing, with the trade-offs stated. Read these when a result surprises you, when you are deciding what to freeze as a gate, or before you repeat a number from a run anywhere it might be mistaken for product evidence.

- [How a run decides](explanation/how-a-run-decides.md): why gates come before the objective, and why the comparison is exact
- [Six loops that were already run](explanation/case-studies.md): real ledgers from bundle bytes, accessibility, an integration contract, SEO, UI defects and answer-engine readiness
- [What a keep does not prove](explanation/limits.md): the claims a green run does not support
- [Architecture](explanation/architecture.md): the crates, and how Git isolation and the protocol fit together

Dated project records are kept for provenance rather than for reading order: the [platform design](project/plans/2026-09-11-autoresearch-rust-platform-design.md), the [README ledger](project/ledger/readme-autoresearch.md) and the [launch notes](project/social/2026-09-14-autoresearch-rs-launch.md).
