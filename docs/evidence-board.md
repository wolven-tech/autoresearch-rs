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

## Redacted export

`autoresearch export --run-id RUN_ID --export-root /absolute/existing/review-dir`
creates a new `autoresearch-RUN_ID` directory outside the product repository.
It refuses an existing destination and never overwrites a prior bundle. Add
`--artifact RELATIVE_PATH` for each report-declared artifact to copy; default
is no artifact copy. `--evidence-root ROOT --receipt SIDECAR.json` may import
selected commercial receipt declarations into the report, but does not copy
receipt source files or fetch remote URIs.

Bundle contains `report.json`, `report.html`, optional `artifacts/`, and
`provenance.json`. Manifest records original frozen-contract SHA-256,
environment fingerprint when available, selected artifact paths, and byte
hashes for each member. Full frozen config, program prompt, journal, raw
process logs, host command record, and ambient credentials stay out. Report
strings are sanitized for common token/API-key patterns; sensitive structural
identifiers and artifact bytes are rejected rather than silently changed.
Symlinked, absent, oversized, or undeclared artifacts are refused.

Redaction is bounded defence, not proof that every possible secret is absent.
Review bundle before sharing. Failed exports may leave a hidden `.pending`
directory under chosen export root; no incomplete bundle is presented as
complete.
