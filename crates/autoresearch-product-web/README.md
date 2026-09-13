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
