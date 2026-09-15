# First loop example

The contract, page and evaluator used by [your first loop](../../docs/tutorials/first-loop.md). Follow that tutorial rather than running these files in place: a loop needs its own repository, and this directory is part of the autoresearch-rs checkout.

| File | What it is |
| --- | --- |
| `site/index.html` | a landing page with two paragraphs and a call-to-action link |
| `evaluator/main.rs` | a standard-library Rust evaluator that counts paragraphs and checks the link |
| `autoresearch.toml` | the contract: objective `paragraph_count` to minimize, hard gate `cta_present` |
| `program.md` | the brief, frozen at baseline alongside the manifest |

The evaluator ships as source, not as a binary. The tutorial compiles it with `rustc` and puts the absolute path into the manifest, replacing the `/ABSOLUTE/PATH/TO/first-loop-evaluator` placeholder, because an evaluator runs with an empty environment and cannot search `PATH`.

The lesson ends with one candidate kept and one discarded: the discarded one removes the call-to-action, which scores better on the objective and fails the gate.
