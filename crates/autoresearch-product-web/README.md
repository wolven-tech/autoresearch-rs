# Product-web evidence adapter

Local Chromium captures frozen routes at 320, 390, 768, and 1280 CSS pixels.
It records document status, console/runtime error counts, horizontal overflow,
unnamed controls, screenshots, reduced-motion media state, bounded keyboard
traversal, detectable focus indicators, and solid-colour text contrast samples.
The JSONL executable uses the Phase 3 subprocess contract and only accepts
manifest-declared loopback targets; off-origin page requests are blocked.

Accessibility probes are **not a WCAG AA audit or certification**. Keyboard
traversal checks at most eight controls; custom focus treatment may be marked
unavailable or misclassified. Contrast sampling only handles simple opaque
foreground and background colours in a bounded DOM sample; gradients, images,
transparency, complex inheritance, and other content require manual review.
`unavailable` never means pass. Screenshots and observations are lab evidence,
not commercial validation or grounds to move a product bet gate.

Run `cargo test -p autoresearch-product-web` with local Chromium available on
`PATH` or `CHROMIUM_PATH`. Browser tests skip when Chromium is unavailable;
CI must supply it for a meaningful browser gate.

Lighthouse import is separate from live execution. `web.lighthouse` freezes
version, a 64-character environment fingerprint, warm-up count, measured count,
and up to eight allowlisted fields. Each run-owned JSON artifact wraps a
standard Lighthouse report as
`{"environment_fingerprint":"…","report":{…}}`. Import rejects changed
version, fingerprint, route, missing declared fields, unsafe paths, and
non-finite values. Warm-ups are validated but excluded from median; measured
raw samples remain in evidence. Category scores use 0–100; FCP/LCP/TBT use
milliseconds; CLS remains unitless. INP is deliberately absent from this lab
adapter. No Lighthouse binary is bundled or silently substituted with another
score; missing tool means unavailable live measurement.

Technical SEO probing is local-only. `[web.seo]` freezes robots and sitemap
paths plus redirect bound; each route declares expected HTTP status and whether
it should be indexable. The adapter checks canonical, noindex, sitemap entry,
robots sitemap declaration, JSON-LD syntax, and declared redirect chains. Each
issue carries exact route, URL, rule, and run-owned JSON artifact. It does not
claim Google indexation, rankings, AI citation, or traffic. Remote production
checks are unsupported until a separate read-only origin allowlist is designed;
off-origin and undeclared redirects are rejected before a second request.
