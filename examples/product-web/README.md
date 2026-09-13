# Product-web fixture

Three offline routes: `/` (normal), `/metadata` (metadata), and `/missing`
(expected 404). `/robots.txt` and `/sitemap.xml` supply technical SEO fixtures.
Pages have no external dependencies. `autoresearch.toml` freezes loopback
origin, route/status/indexable policy, exact 320/390/768/1280 CSS widths,
reduced-motion mode, SEO paths, GEO entity/facts/passage bound, and Lighthouse
version/fingerprint/field/sample rubric. Changing those inputs changes frozen
identity; it does not move any product bet gate.

From repository root, with Chromium on `PATH` or `CHROMIUM_PATH` set:

```text
cargo test -p autoresearch-product-web --test product_web_pack offline_product_web_pack_runs_all_adapters_through_phase3_validator
cargo test -p autoresearch-product-web --test browser
cargo test -p autoresearch-product-web --test seo
cargo test -p autoresearch-product-web --test geo
cargo test -p autoresearch-product-web --test lighthouse
```

Named pack test starts a disposable loopback server and runs rendered browser,
responsive, bounded accessibility, SEO, GEO, and frozen Lighthouse **import**
through Phase 3 structural and manifest/artifact validators. It expects four
screenshots, one SEO JSON artifact, one GEO JSON artifact, four synthetic
Lighthouse report artifacts (one warm-up plus three measured), and fixture
score 82. Synthetic reports test importer, not real performance. Test fails
if Chromium is missing. Separate `browser`, `seo`, `geo`, and `lighthouse` tests
exercise failure paths. Optional `cargo run -p autoresearch-product-web
--example product_web_fixture` serves static fixture at `127.0.0.1:4419` for
manual inspection; stop with Ctrl-C.

Accessibility output samples keyboard traversal, focus and solid-colour
contrast. It cannot certify WCAG AA: manually review full keyboard journey,
assistive technology, responsive reflow, meaningful imagery, dynamic content,
and complex colour treatments. SEO/GEO lint does not prove indexation, live AI
citations, demand, payment, or any promotion/kill gate. Production probing is
absent from offline pack and requires separate exact allowlist plus per-run
network authority.
