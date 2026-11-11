# ritual-full replaces AllStack

Planned work in this repo now runs as one `ritual-full` flow: Think, Spec, Plan, Build, Review,
Ship and Reflect on a Chronis epic. The design approval is the user's decision. The AllStack
native skills, the managed-execution workflows and the reference docs that came with them are
removed. The dated AllStack-era plan and verification records stay as history.

Guide: [ritual, zero to hero](https://github.com/decebal/decebal-claude-skills/blob/main/docs/ritual-zero-to-hero.md).

## Approval model

- Planned work: one `ritual-full` flow on a Chronis epic. The epic's `ritual-*` lines name each
  step's file.
- Design approval: the user's decision. An agent runs
  `cos ritual approve <epic> <design> --by "<name>" --review <path> --yes`, or `cn approve <epic>`
  without `cos` once the user has appended the `ritual-approved:` line (`ritual/references/spec.md`,
  Approve), only after the user explicitly tells it to approve in the conversation, quotes that
  instruction in its report, and never approves on its own initiative.
- `--yes` needs decebal/chief-of-staff-v2#23.
- Enforcement is an instruction in `CLAUDE.md` and in the vendored `spec.md`. No hook in this repo
  enforces it: `.claude/` tracks only skills.
- A bug with a clear reproduction skips Think and Spec and goes through `systematic-debugging`.

## What changed

| Area | Before | After |
|---|---|---|
| Workflow skills | 9 `allstack-*` skills in `.claude/skills/` and 9 in `.agents/skills/` | `ritual`, `ritual-full`, `semantic-isa` in both directories |
| Managed execution | `workflows/`: `bugfix.toml`, `catalog.toml`, `delivery.toml`, `verification.toml`, 4 files in `methods/` (8 files) | removed |
| Reference docs | `docs/reference/allstack.md`, `allstack-skills.md`, `allstack-catalog.md` | removed; their index lines in `docs/README.md` removed |
| Ignored store | `/.allstack/` in `.gitignore` | line removed. The main checkout holds `.allstack/prime/` (`storage/`, `wal/`, `prime.lock`); it shows as untracked after merge. This PR does not remove it |
| Planning instructions | `CLAUDE.md` had no planning routing | `CLAUDE.md` gains a Planning section with the approval model above |
| Index | `docs/README.md` listed three AllStack reference pages and a "Native Allstack delivery" section | one line pointing at this record; the AllStack-era records are listed as history |

The vendored skills are byte-for-byte copies of `skills/ritual`, `skills/ritual-full` and
`skills/semantic-isa` at decebal-claude-skills PR #97 head `8af7330`
(`feat/agent-approval-when-told`), in both `.claude/skills/` and `.agents/skills/`. That PR is not
on the repo's main (`1cd0492`). Its approval text differs from main: main's `spec.md` and
`ritual/SKILL.md` describe the approval differently. When #97 merges, re-vendor from main and diff.
`ritual-full` links `../ritual/` and `../semantic-isa/`, so all three travel together.

## Kept on purpose

| Path | Why |
|---|---|
| `docs/plans/allstack-c80b8084-4d2e-491d-8051-bb3c4d997aa1.md` | Dated delivery plan record. History, not a live route. Linked from the docs index |
| `docs/project/plans/2026-10-25-manual-agent-readiness-request.md`, `...-plan.md` | Dated plan records from the AllStack trial. History |
| `docs/project/reports/2026-10-25-native-manual-agent-readiness-verification.md` | Dated verification record. History. Its link to `workflows/verification.toml` is now plain text, because this PR removes that file. Its line 57 says receipts remain under ignored `.allstack/`; that is no longer true after this PR |
| `.planning/` | Phase records for the autoresearch build. Mentions `.github/workflows/ci.yml`, which is GitHub Actions, not AllStack |
| `.github/workflows/ci.yml` | Repo CI, unrelated to AllStack |

## Preflight rows for this repo

Run 2026-11-11 from the main checkout.

| Check | Command | Result |
|---|---|---|
| Chronis ready | `cn ready --toon` (run from the main checkout) | 0 ready (empty listing) |
| Chronis open tasks | `cn list --toon --all` (run from the main checkout) | 0 with status `open`; every listed task and epic is `done` |
| graphify | `ls graphify-out` (run from the main checkout) | absent, so no graph to query; graphify is not on the preflight path for this repo |

Machine-level install state is not recorded in this repo.

## Checks run

- Removed-path search: `git grep -i -E 'allstack|workflows/(bugfix|catalog|delivery|verification)|methods/'`.
  Excluding the kept records (table above) and this record, one hit: `docs/README.md:47`, the
  index line that names those records as history. No code, CI, hook, or test reference to the
  removed paths or the `allstack` CLI.
- `diff -r` of `ritual`, `ritual-full` and `semantic-isa` against the `8af7330` source, for both
  `.claude/skills/` and `.agents/skills/`: 6 of 6 identical, no output.
- `AGENTS.md`: gitignored (`.gitignore:17`), not tracked, not generated in this repo. No
  `agent-docs.toml` exists, so no `render-agent-docs` step applies.
- Repo code gates (fmt, clippy, test, doc): not run, text-only change. Listed unchecked in the PR
  test plan.

## What I have NOT checked

- No ritual-full flow has run end to end in this repo. The first planned feature after merge is
  that test.
- `ritual/SKILL.md` and `ritual-full/SKILL.md` link `../../docs/ritual-zero-to-hero.md`. That path
  resolves only inside decebal-claude-skills, so the local copies carry a dead relative link. The
  guide link in this record is the working one. Accepted: kept byte-identical to upstream.
- `semantic-isa` enhances `arch`, `security-review`, `rust-clean-architecture` and
  `integration-test` (`.agents/skills/semantic-isa/SKILL.md:12`). None of the four is carried under
  `.claude/skills/` or `.agents/skills/`, so a session running semantic-isa's DESIGN mode goes
  without them. `ritual/references/review.md:31` also names `security-review`, so the review step's
  security item has no local skill either.
- `cos ritual approve --yes` has not been exercised in this repo. It depends on
  decebal/chief-of-staff-v2#23.
- The `ritual-goldfish`, `builder` and `reviewer` roles that `cos agents spawn` hires were not
  checked against a local cos workspace.
- The removed `workflows/` files were not replayed. Any run that had been started against them
  is not recovered here. This PR touches no `.allstack/` state, but the main checkout's
  `.allstack/prime/` becomes untracked-visible after merge (see the Ignored store row). Left in
  place; removing it or excluding it locally is the user's call.
- Text-only change: nothing was compiled or gated.
