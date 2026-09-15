# First Loop: Page Content Quality

## Objective

Minimize the number of paragraphs on the landing page while keeping the call-to-action link visible.

## Scope

Edit only files under `site/`.

## Gates

1. **cta_present** — The CTA link (class="cta") must exist on the page.

## Metrics

- **paragraph_count** — Number of `<p>` elements on site/index.html. Lower is better because simpler content is clearer.

## Context

This is a learning loop that demonstrates autoresearch-rs on a tiny static site. The evaluator counts paragraphs and verifies the call-to-action is still present. Each candidate can edit the HTML to add, remove, or modify content. The decision rule is straightforward: improve the paragraph count or discard; if tied, the tie-breaker uses lines changed.

## What you control

You or an agent can edit `site/index.html` to:
- Add or remove paragraphs
- Change text within paragraphs
- Modify the page structure

You cannot:
- Delete or move the CTA link
- Edit files outside `site/`
- Change `autoresearch.toml` or `program.md`

After editing, autoresearch commits your changes and runs the evaluator. The decision is final: the result either keeps the candidate and moves the local branch, or discards it and deletes the worktree.
