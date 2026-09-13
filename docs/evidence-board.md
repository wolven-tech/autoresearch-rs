# Evidence board

`autoresearch report --run-id RUN_ID` rebuilds versioned JSON from frozen
inputs and append-only journal. Add `--html` to emit one self-contained HTML
document to stdout. Neither form runs evaluators, selects a candidate, edits
the target checkout, deploys, or publishes anything. The HTML has inline CSS,
no scripts, no remote fonts, and no network dependency. Save stdout under a
review-controlled path when a persistent copy is needed.

The board shows baseline and retained-best objective, exact candidate order,
frozen hard-gate outcomes, changed paths, runtime, decision reason, and
run-owned artifacts. A failed evaluator attempt is shown as a typed failure,
not as a score. Missing baseline artifacts and legacy changed-path fields are
labelled unavailable. Links appear only for existing, non-symlink artifact
files under the run's `artifacts/` directory; remote receipt URIs are text,
not outgoing links.

Commercial receipts remain in a separate section. A matching SHA-256 for a
local file verifies bytes against the operator's declaration; it does not
verify payment, delivery, customer use, or a product promotion gate. Internal
web scores and synthetic fixtures cannot establish commercial validation.

Browser fixture covers 320, 390, 768, and 1280 CSS pixels, horizontal
overflow, keyboard focus, native evidence disclosure, and reduced motion.
These checks are not a blanket WCAG AA certification. Evaluate any exported
board with real assistive technology before relying on that claim.
