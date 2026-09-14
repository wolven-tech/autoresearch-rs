# autoresearch-rs social launch

Status: published on 2026-09-14.

- [LinkedIn post](https://www.linkedin.com/feed/update/urn:li:activity:7505300232457539584/)
- [X thread](https://x.com/ddonprogramming/status/2099564536881647944) (seven posts)

Audience: developers and technical founders running product-web or small model
experiments. Voice: personal founder account; concrete case first, no hype.

Repository `main` was pushed and verified before the X thread was published.

Visual for both posts: [3D experiment gate](../../assets/social/autoresearch-rs-experiment-gate-2026-09-14.png).
Attach to LinkedIn post and first X post. This is concept art, not a benchmark
or screenshot.

Alt text: 3D illustration of two metal experiment modules approaching a fixed
glass evaluation gate. One rests in an illuminated keep cradle; another sits
in a shadowed discard tray. No scores or measured results appear in the image.

## LinkedIn

In a disposable product-page fixture, a candidate improved the score and broke
the CTA. It was discarded.

That is the failure autoresearch-rs is built to catch.

I've been building a Rust fork of Karpathy's autoresearch for product work and
small training experiments. The upstream Python source and provenance stay in
the repo.

The loop freezes the evaluation contract before the baseline. Each candidate
gets an isolated Git worktree, exact-commit evaluation, hard gates, and a
keep-or-discard decision. Interrupted runs have a journal. Reports show what
changed and why a candidate stayed or went.

Current work covers local browser routes, bounded accessibility checks,
SEO/GEO diagnostics, and a tiny CPU/Candle training fixture.

The boundaries matter. The fixture is not full nanochat parity. Accessibility
sampling is not WCAG AA certification. SEO/GEO checks do not prove rankings or
AI citations. No lab score proves customer demand.

I want better experiments, not better-looking dashboards.

What hard gate would you freeze before optimizing your next product or model?

https://github.com/wolven-tech/autoresearch-rs

## X thread

### 1/7

In a disposable product-page fixture, a candidate improved the score and broke
the CTA. It was discarded.

That is the point of autoresearch-rs: freeze what must not break before
optimizing.

### 2/7

I forked Karpathy's autoresearch into a Rust experiment engine. Upstream Python
source and provenance stay in the repo. Rust owns frozen contracts, isolated
Git worktrees, exact-commit evaluation, journal recovery, and reports.

### 3/7

Decision order: hard gates first, primary objective second. A higher score
cannot compensate for a failed gate. Candidate changes cannot quietly rewrite
evaluators, thresholds, or allowed files after baseline.

### 4/7

Product-web lane: local Chromium routes, responsive and accessibility samples,
SEO/GEO diagnostics, and frozen Lighthouse report import. Engineering checks,
not WCAG AA certification, live rankings, or AI citations.

### 5/7

Training lane: tiny CPU/f32 Candle fixture with frozen data and a numerical
comparison to Python. Useful experiment plumbing; not full upstream nanochat
parity, CUDA throughput, or an H100 benchmark.

### 6/7

Reports tie each keep/discard decision to an exact commit, changed paths, gate
outcomes, and artifacts. Recovery does not guess after an interrupted Git
effect.

### 7/7

Lab scores never move a product bet gate. Customer evidence stays separate.

Code and docs: https://github.com/wolven-tech/autoresearch-rs

What gate is missing from your current experiment loop?

## Claim checks

- [README](../../README.md): frozen contracts, worktrees, exact commits,
  recovery, reports, and explicit limits.
- [Product-web fixture](../../examples/product-web/README.md): local browser,
  accessibility, SEO/GEO, and synthetic Lighthouse import boundaries.
- [Training parity](../training-parity.md): exact tiny-fixture scope and known
  discrepancy; no full nanochat or hardware parity claim.
- [Portfolio packs](../../examples/portfolio/README.md): templates, not live
  product runs or commercial evidence.
- [Evidence board](../evidence-board.md): report content and redacted export.

## Art provenance

Generated with built-in imagegen on 2026-09-14. Final prompt:

> Use case: ads-marketing. Asset type: one shared landscape image for LinkedIn
> and X launch posts, wide 16:9 composition with safe central crop. Create
> distinctive high-end 3D editorial art for autoresearch-rs, a Rust experiment
> engine whose core idea is freezing an evaluation contract, testing one bounded
> candidate, then keeping or discarding it against the same gate. Quiet
> precision lab on a dark neutral plane, no people. A compact transparent
> glass measurement fixture physically fixed in place; two small machined
> metal modules travel on separate precise rails toward it, representing
> baseline and candidate. One module is retained in a clean illuminated
> cradle after the fixture; one is visibly diverted to a dim side tray. Make
> the fixed gate and keep/discard fork legible at thumbnail scale. Premium
> tactile 3D render, architectural precision, restrained, no sci-fi interface.
> Brushed metal, clear glass, subtle warm copper accents and graphite base.
> Soft directional studio lighting, crisp shadows, believable reflections.
> Oblique isometric view, strong silhouette, generous clean space for crops.
> Metaphorical illustration, not a claim about benchmarks or customer results.
> No words, letters, numbers, labels, logos, charts, screenshots, fake UI,
> people, robots, gradient blobs, confetti, or watermark.
