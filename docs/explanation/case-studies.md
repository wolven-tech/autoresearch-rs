# Six loops that were already run

Every number here comes from a ledger of a loop I ran before this CLI existed, by hand or through my autoresearch skill, against the same contract the CLI now enforces. Each case says what was frozen, what moved, and what the CLI covers today versus what you still write. None of them proves the tool works; they show what the shape of a measured loop is on a product surface you probably own.

| Lane | Scalar | Better | Baseline | Best |
| --- | --- | --- | --- | --- |
| [Bundle bytes](#bundle-bytes-wasm-size-while-adding-components) | release wasm bytes | lower | 510,535 B | 423,713 B |
| [Accessibility](#accessibility-axe-violations-plus-unresolved-contrast) | axe violations plus unresolved contrast | lower | 5 | 0 |
| [Integration contract](#integration-contract-behaviours-nobody-asserts) | unasserted behaviours | lower | 21 | 6 |
| [SEO](#seo-score-on-one-long-post) | `seo_score` on a local scorer | higher | 52.4 | 95.9 |
| [UI defects](#ui-defects-on-an-app-timeline-screen) | weighted defect count | lower | 29 | 0 |
| [Answer-engine readiness](#answer-engine-readiness-geo) | `retrieval_readiness_score` | higher | 5.00 | 100.00 |

## Bundle bytes: WASM size while adding components

The corpus was a component audit of one real marketing page: nav with call-to-action, hero with dual actions, three-column numbered feature grid, numbered process steps, itemised cost breakdown, one-off pricing block, FAQ and multi-column footer. The scalar was the `apps/web` release wasm in bytes, lower is better. The gate was cargo build, test, clippy `-D warnings`, fmt, `cargo audit --deny warnings`, the wasm32-unknown-unknown cross-compile, and `apps/web` still compiling as the coverage fixture. A rendered visual check joined the gate after a blank page passed every command.

Baseline `510,535 B`, best `423,713 B`. The ledger's net line and its only discard:

```text
−86,822 B (−17.0%) against baseline, while adding 20 components.

| 5 | `opt-level = "z"` in `[profile.release]` | 490,481 B | 0 | **Discard** — byte-identical, because `dx` builds the bundle with its own `wasm-release` profile. Everything tuned in `[profile.release]` was reaching the server binary and never the browser. |
```

The interesting rows are the ones the number could not see. The Tailwind stylesheet shipped as 23 bytes, fixed in row 2, which moved the scalar by 0 bytes. A blank page threw an `atob` error, fixed in row 7, which was also the largest single drop at `−59,956 B` against row 6. "Compiles" was not "renders", so a rendered check joined the gate. The loop stopped on effort budget, and its 2-consecutive-discard stop never tripped.

Two of those rows would behave differently under autoresearch-rs today. Row 2's fix moved the objective by 0 bytes, which is a tie, and a tie is discarded unless it changed fewer lines than the current best or, on equal lines, ran faster, so a real fix the number cannot see belongs in a gate. Row 5 changed `[profile.release]`, which normally lives in `Cargo.toml`, and a candidate touching `Cargo.toml` cannot be decided by the CLI today.

The cargo hard-gate adapter in `autoresearch-evaluator` knows fmt, clippy, test and build, but it is a library API that no manifest can select, and it expects `CARGO_NET_OFFLINE` and `CARGO_TARGET_DIR` in a declared environment the CLI never passes. Through the CLI, your JSONL evaluator runs those gates itself, reports each one as a hard gate, and prints the byte count. The [performance pack](../../examples/portfolio/performance/autoresearch.toml) is a template whose evaluator placeholder must be replaced. Source ledger: [component-kit-autoresearch.md](https://github.com/wolven-tech/rust-v2/blob/main/docs/ledger/component-kit-autoresearch.md).

## Accessibility: axe violations plus unresolved contrast

The corpus was the `/motion` page served by `apps/web`, at 1280×900, plus its `prefers-reduced-motion: reduce` state. The scalar was violation nodes plus unresolved color-contrast nodes, as reported by axe-core 4.10.2 under `wcag2a`, `wcag2aa`, `wcag21a` and `wcag21aa`, lower is better. The gate was `cargo xtask ci` green, all seven components present and interactive, no component deleted, every infinite animation covered by `prefers-reduced-motion`, no unsubstituted template placeholder in rendered text, added mid-loop, and a screenshot that looks right.

| Row | Scalar | What it counted |
| --- | --- | --- |
| baseline | 5 | one `html-has-lang` violation, four unresolved contrast nodes |
| best | 0 | nothing open under the four tag sets |

The first version of the scalar counted violations alone and read 1, which looked almost compliant. Counting axe "incomplete" nodes as open questions gave the real 5. Row 1b was a regression the gate missed: the `{script_include}` placeholder rendered as literal text at the bottom of every page, it was caught only by a screenshot, and the gate was strengthened. A zero here is not WCAG AA conformance; the [limits page](limits.md) says why.

The product-web browser adapter samples bounded keyboard traversal and solid-colour contrast in local Chromium, but it does not run axe-core, it emits hard gates only, and it wants `CHROMIUM_PATH` set for it, which no CLI run does, so through the CLI it reports "Chromium path unavailable". A numeric axe count is your own JSONL evaluator, which launches the browser by absolute path. The "looks right in a screenshot" gate has no slot in the protocol unless your evaluator can compute it, so keep that read human. Source ledger: [motion-aa.md](https://github.com/wolven-tech/rust-v2/blob/main/docs/ledger/motion-aa.md).

## Integration contract: behaviours nobody asserts

The corpus was 21 behaviours of the AllSource Core integration. The scalar was corpus behaviours with no direct automated assertion against a live Core, lower is better. The gate was cargo build, test, clippy `-D warnings`, fmt `--check`, `cargo deny check`, `cargo machete`, the wasm32 cross-compile, and all three vertical-slice tests against a live Core.

The ledger's net line:

```text
21 → 6 unasserted (−71%). 13 contract tests, all running in CI.
```

The gate rejected proposal 3's first draft, and that rejection was the finding. The draft asserted that Core normalizes a PascalCase event type, and it failed with `append failed: 400`. Normalization is client-side only: the belief was wrong, not the code. A newly discovered protective behaviour was deliberately left out of the corpus mid-loop, because adding it would have let the loop change its own denominator. Under autoresearch-rs, that corpus file belongs in `protected_paths`, carved out of the mutable root.

autoresearch-rs covers keeping the corpus out of reach of candidates, the isolated worktree per candidate and the gate-first decision. The cargo gate list and the count are your JSONL evaluator, for the same empty-environment reason the bundle case gives. If your Core runs off the machine, `requires_network = true` also needs `network` in `authority.allow`, that declaration isolates nothing, and such a manifest can only run in manual mode. Source ledger: [allsource-integration-corpus.md](https://github.com/wolven-tech/rust-v2/blob/main/docs/ledger/allsource-integration-corpus.md).

## SEO score on one long post

The corpus was one newsletter post, Rust AI Weekly #12, scored by a read-only local script. The scalar was `seo_score`, 0 to 100, higher is better. The gate was: no change to slug, filename, or any fact, number, verdict, name or date; series format and no em dashes preserved; keyword density capped at 2%; a change that improves the score but reads like SEO copy is a discard; no app code touched inside the loop; a human reads the diff before push; a budget of 20 experiments or 5 without movement.

Baseline `52.4` (4079 words), best `95.9` (4086 words), on the local scorer only. Three of the discards:

```text
f841cd7	95.9	4086	discard	add "gpu" tag
1f694e0	88.0	4086	discard	longer seoTitle naming all three hooks
b488d29	86.1	4086	discard	shorter seoDescription variant (control)
```

Every discard also tied or lost on score, so the score alone rejected each one. The "gpu" tag tied at 95.9, and a tie is a discard in that ledger. Under the CLI the same tie would be settled on changed lines against the kept 95.9 candidate, and a one-line tag could win it, so if a tag is not worth keeping, say so in a gate. Checks the series format fails by design, such as an average sentence of 30.5 words, were left failing on purpose and written down. This is a local scorer, and it proves no ranking.

The [SEO pack](../../examples/portfolio/seo/autoresearch.toml) is a template, and the product-web SEO checks produce diagnostics and artifacts, not a number. A numeric score needs a JSONL wrapper around your scorer. The "reads like SEO copy" gate and the human read of the diff stay with you; nothing leaves the `autoresearch/<run-id>` ref until you merge it.

## UI defects on an app timeline screen

The corpus was a Dioxus web app's root at 1440×900, 390×844 and 320×844, with four stage jumps, first-load resource order, scroll, text spacing, keyboard focus and reduced motion. The scalar was a weighted defect count, lower is better:

| Defect | Weight |
| --- | --- |
| stylesheet discovered behind WASM | 8 |
| product font not preloaded | 3 |
| sticky backdrop-filter compositor risk | 5 |
| rail/header container misalignment | 4 |
| floating connector dots | 4 |
| no selected-stage state | 3 |
| hover position movement | 2 |
| each unresolved Axe contrast node | 1 |
| rail height above 100px | 10 |

Baseline 29, best 0. Two rejected rows, in their ledger:

```text
| 1 | Critical CSS/font plus contained timeline and content-crossing shimmer | 15 | Reject — Axe marked all 15 rail text nodes contrast-indeterminate. |
| 3 | Replace pseudo track with explicit aria-hidden timeline element | 10 | Reject — generic `li` rule made track sixth grid item and expanded desktop rail to 160.6px. |
```

Three candidates improved the scalar, to 15, 15 and 10, and were still rejected, because each left a named defect behind. Only row 4, at 0, was kept. After it, browser resource entries started the font at 34.0ms and CSS at 34.1ms, before WASM at 37.4ms and first paint at 56ms.

This is the case the README opens on. In autoresearch-rs terms each rejection is a better-scoring candidate lost to a failed hard gate, so the contrast and rail-height defects belong in named hard gates, not only in the weights. The [ui pack](../../examples/portfolio/ui/autoresearch.toml) is a template, and the product-web route and accessibility checks supply hard gates as library code. The weighted count is your JSONL evaluator.

## Answer-engine readiness (GEO)

This loop ran on ChargeWindow, my own product. The scalar was `retrieval_readiness_score`, higher is better, and `geo_readiness_score` stayed at 100.00 on every row as a regression gate. At baseline, production already returned HTTP 200 to both Perplexity agents, but the source lacked explicit access, canonical-domain corpus alignment, namesake disambiguation and active discovery submission.

| Row | retrieval_readiness_score |
| --- | --- |
| baseline | 5.00 |
| crawler-access | 18.00 |
| canonical-domain | 28.00 |
| entity-resolution | 83.00 |
| sitemap-freshness | 88.00 |
| answer-corpus | 93.00 |
| discovery-path | 100.00 |

The production IndexNow row is logged `submitted`, not `keep`, with the note `acceptance does not prove indexing or Perplexity inclusion`. Entity resolution was the largest step, and that size comes from the evaluator's own weights, not from observed retrieval: crawler-access was 13 of 100 points, and entity-resolution was 55 points across five checks. I could not tell readiness work apart from actual citations. The score measures readiness, and it says nothing about indexing, ranking or citations.

The [geo pack](../../examples/portfolio/geo/autoresearch.toml) and the product-web entity, passage and source-coverage diagnostics cover the local lint, and linked sources are not fetched. A 100-point regression gate becomes a boolean hard gate your evaluator computes. The product-web production probes cannot be enabled through the CLI. Nothing stops your own evaluator from reaching production, so keep production calls out of it yourself.

## Other surfaces the contract has run on

The same contract has also run on conversion rubrics on application pages, read-only go/no-go research screens that were allowed to say no, first-paint and TTFB budgets, and a correctness-first then runtime loop on a number parser.

To turn one of these into a contract you can run, start from [write an evaluator](../how-to/write-an-evaluator.md) and the [manifest reference](../reference/manifest.md). For what any of these numbers does not settle, read [what a keep does not prove](limits.md).
