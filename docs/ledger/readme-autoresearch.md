# README autoresearch ledger

The README was rewritten for product engineers and then reduced in formatting slop through two autoresearch-rs runs, scored by the `readme-lab` evaluator in [tooling/readme-lab](../../tooling/readme-lab). Both kept commits were re-verified with `verify`, and both reported `matched`.

| Measure | Original README | After run A | After run B |
| --- | --- | --- | --- |
| Use-case lanes with verified evidence | 0 of 6 | 6 of 6 | 6 of 6 |
| Dev slop points (51 rules) | 8 | 2 | 0 |
| Holdout slop points (17 rules) | 0 | 0 | 0 |
| Prose words | 404 | 3,991 | 3,999 |
| Code blocks | 3 | 19 | 14 |
| Lines | 99 | 468 | 354 |

A lower slop count is a claim about formatting patterns the rules know. It is not proof that the README reads well, and the rules were tuned on a small corpus (below).

## Setup

The frozen corpus is the six use-case lanes in the README, each taken from a measured ledger of my own loops. Every number and quoted row was checked byte for byte against its source file by an adversarial verifier before drafting, and five of the six lane summaries needed corrections. Client work is excluded.

The evaluator speaks the JSONL protocol and has two contracts:

| Contract | Objective | Hard gates |
| --- | --- | --- |
| `coverage` (run A) | `uncovered_use_cases`, minimize | `links_resolve`, `cli_invocations_parse`, `code_blocks_parse`, `limits_preserved`, `markdown_well_formed` |
| `slop` (run B) | `slop_points` from `rules/dev.toml`, minimize | the five above plus `use_cases_covered`, `substance_floor` (at least 2,800 prose words and 8 code blocks), `holdout_not_regressed` (holdout points at most 0) |

`cli_invocations_parse` parses every shell example with the real `autoresearch` clap definition, and `code_blocks_parse` validates every manifest block with the real `ValidatedManifest` parser. `limits_preserved` requires seven stated limits (WCAG, rankings and citations, customer evidence, training parity, process isolation, no deploy or publish, not autonomous) and rejects clause-level overclaims of them.

The slop rules come from a catalog of 68 formatting patterns. They were split deterministically: within each category, ids are sorted and every third one goes to `rules/holdout.toml`, which the proposer never read. Each rule carries an `example_bad` and an `example_good`, and `readme-lab selftest` checks that it fires on the first and stays silent on the second.

Before the runs, three adversarial agents attacked the evaluator. One gamed the rules, one hunted false positives on 13 published READMEs (ripgrep, uv, hyperfine, just, xsv and others), and one tried to get inaccurate READMEs past the gates. They filed 73 findings, and an independent verifier confirmed 53 fixes. The frozen binary's SHA-256 is `5510156a15cb3976600704f05b5398010694552b49538c450bd188f4c48bccfd`. A debug build embeds its source paths, so rebuilding the same source in this repository gives different bytes (`5d43ca3e…56d496c` here) and the same scores on the same README.

## Run A: coverage

Run `run-1789429245771-35145-0645c9ee`, base commit `0645c9e`, frozen identity `dd099da7…44a36e`.

| # | Hypothesis | Gates | `uncovered_use_cases` | Slop (diagnostic) | Decision |
| --- | --- | --- | --- | --- | --- |
| 0 | Original README | all pass | 6 | 8 | baseline |
| 1 | Rewrite for product engineers with six measured lanes, a real CLI decision demo and code-verified caveats | all pass | 0 | 2 | keep, `primary_improvement` |

Candidate 1 was not written inside the loop. Three drafts were written from different angles, judged by three independent lenses (product engineer, accuracy against source, formatting slop), synthesized, fact-checked sentence by sentence against the code, and put through its quickstart end to end in a disposable repository. The loop decided on the result; it did not produce it.

## Run B: formatting slop

Run `run-1789429354023-41886-54a59af2`, base commit `54a59af`, frozen identity `f287496d…f5b40e`.

| # | Hypothesis | Gates | `slop_points` | Holdout | Decision |
| --- | --- | --- | --- | --- | --- |
| 0 | Run A's kept README | all 8 pass | 2 (`wall-of-code`: 188 code lines, 72 prose lines) | 0 | baseline |
| 1 | Keep one worked manifest fragment (bundle lane) and drop the five repeated per-lane fragments | all 8 pass | 0 | 0 | keep, `primary_improvement` |

The run was stopped with `stop` once the objective reached zero, since no candidate can improve on it.

## After the runs

Commit `1ff2071` landed upstream while the runs were in progress. It adds [skills/autoresearch](../../skills/autoresearch/SKILL.md) and a README section about it. The kept README was rebased onto that commit by hand, and the skill section was folded into "Driving it from an agent" as a `### The Claude skill` subsection. That edit was scored outside a run, so it is a check and not a loop decision: all 8 slop-contract gates pass, dev slop is 0, holdout slop is 0, with 4,094 prose words and 15 code blocks.

A second pass added attribution, a LICENSE and NOTICE, a lane table and reader paths in the opening, a mermaid loop diagram, a Status section, and three badges (CI, license, Rust toolchain). The badges went in only after CI on `main` was green again, and every badge URL was fetched and returned an SVG. That pass was also scored outside a run: all 8 gates pass, dev and holdout slop are both 0, with 4,354 prose words and 16 code blocks.

## What is still open

The residual cases of `limits_preserved` are semantic. It catches the overclaim forms in its patterns and seven known lies, but a contradiction worded outside those patterns can still pass while the limit is stated elsewhere. The final README was fact-checked by an agent for this reason, and that check is not part of the frozen evaluator.

No candidate in either run failed a gate, so neither run shows a gate rejecting anything in the loop. The gates were tested only by fixtures: a negative fixture fails all five coverage gates, and the attack fixtures flip each gate.

The dev rules were tuned against 13 exemplar READMEs and a handful of fixtures. A zero means none of the 51 dev patterns fired, and the holdout zero means none of 17 unseen patterns fired either. Neither says anything about patterns the catalog does not contain.

The README has 3,999 prose words, several times longer than uv's or hyperfine's. No run optimized for length, because deleting caveats would shorten it and nothing gates those caveats yet.

The quickstart manifest points at `/opt/autoresearch/bin/page-weight-evaluator`, which the reader has to build; the README says so. The quickstart run needed a 50-line evaluator written for the purpose.

The evaluator binary is pinned by hash in this ledger only. autoresearch-rs freezes the manifest and program, not the evaluator's bytes.

## Reproduce

Build the evaluator and score a README:

```bash
cargo build --offline --manifest-path tooling/readme-lab/Cargo.toml
tooling/readme-lab/target/debug/readme-lab selftest --rules tooling/readme-lab/rules/dev.toml
tooling/readme-lab/target/debug/readme-lab score --contract slop --config tooling/readme-lab/lab.toml --root . --readme README.md
```

The runs themselves used a disposable clone with `autoresearch.toml` pointing at the frozen binary by absolute path, because evaluators start with an empty environment.
