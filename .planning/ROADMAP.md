# Roadmap: Autoresearch Rust

## Overview

Build safe experiment substrate first, then Git isolation, evaluators, product adapters, mutation agents, reports, Candle training, parity proof, and reusable portfolio manifests. First usable release ends after reporting.

## Phases

- [x] **Phase 1: Foundation** - Workspace, typed decisions, frozen config, journal, and CLI shell.
- [x] **Phase 2: Git Loop** - Isolated worktrees, protected paths, branches, locks, and recovery. (completed 2026-09-12)
- [x] **Phase 3: Evaluator SDK** - Native traits, JSONL protocol, command, Cargo, and diff adapters. (completed 2026-09-13)
- [x] **Phase 4: Product Web** - Browser, Lighthouse, accessibility, SEO/GEO adapters and fixture.
- [x] **Phase 5: Agents and Evidence** - Mutation commands, static report, redaction, and export. (completed 2026-09-13)
- [ ] **Phase 6: Candle and Portfolio** - Candle backend, Python parity, and reusable portfolio packs.

## Phase Details

### Phase 1: Foundation
**Goal**: Compile a strict Rust workspace that validates frozen experiment contracts and deterministic candidate decisions.
**Depends on**: Nothing
**Plans**: 4 plans

Plans:
- [x] 01-01: Workspace quality policy and typed decision core
- [x] 01-02: Manifest schema, validation, and canonical identity
- [x] 01-03: Append-only journal state and recovery model
- [x] 01-04: CLI init, doctor, and baseline shell

### Phase 2: Git Loop
**Goal**: Execute recoverable candidates outside caller checkout with containment enforcement.
**Depends on**: Phase 1
**Plans**: 4 plans

Plans:
- [x] 02-01: Repository validation and exclusive run lock
- [x] 02-02: Run branch and isolated candidate worktree lifecycle
- [x] 02-03: Protected-path and symlink containment enforcement
- [x] 02-04: Journal-driven Git recovery and safe cleanup

### Phase 3: Evaluator SDK
**Goal**: Run trusted native and untrusted subprocess evaluators through one typed contract.
**Depends on**: Phase 2
**Plans**: 8 completed stories in `tasks/prd-phase-3-evaluator-sdk.md`

Native contract, JSONL v1 protocol, strict declared-output validation,
bounded process execution, command/Cargo hard gates, exact-commit diff
evidence, and local examples are complete. Phase 4 Product Web remains next;
`run`/`resume` and reporting still belong to later phases.

### Phase 4: Product Web
**Goal**: Measure browser, accessibility, performance, SEO, GEO, and production properties without moving product gates.
**Depends on**: Phase 3
**Plans**: 9 tracked stories; complete 2026-09-13

Local Chromium route, responsive, reduced-motion, bounded accessibility,
technical SEO, and annotated-source GEO evidence are implemented. Lighthouse
report import validates frozen version/fingerprint/fields and sampling; named
offline pack uses synthetic reports only to exercise import. Optional
read-only HTTPS production probe remains disabled without frozen allowlist,
manifest network ceiling, and per-run permission. Phase 3 validators accept
the combined offline artifact pack. These lab checks cannot establish market
evidence, WCAG AA conformance, live ranking, or AI citations.

### Phase 5: Agents and Evidence
**Goal**: Drive provider-neutral mutations and produce auditable static reports and redacted bundles.
**Depends on**: Phase 4
**Plans**: 13 tracked stories; complete 2026-09-13

Manual and exact-executable-allowlisted command mutations stay in isolated Git
worktrees. Bounded serial scheduling, frozen hard gates, exact-commit decisions,
journal recovery, independent verification, JSON/HTML reports, and selected
redacted local export pass disposable-repository integration tests. A synthetic
product-page loop keeps one valid improvement and discards a higher-scoring
failed-CTA-gate regression after a simulated post-commit interruption. Caller
checkout remains unchanged. Separate Chromium Product Web pack validates local
browser, accessibility, SEO/GEO, and report-import adapters. Neither fixture
is live market evidence, a WCAG AA certificate, or an autonomous product launch.
External writes, deploy, outreach, payment, and gate movement remain denied.

### Phase 6: Candle and Portfolio
**Goal**: Add Candle training, parity fixtures, and reusable weekly-bet manifests after product engine is usable.
**Depends on**: Phase 5
**Plans**: TBD

## Progress

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation | 4/4 | Complete | 2026-09-11 |
| 2. Git Loop | 4/4 | Complete | 2026-09-12 |
| 3. Evaluator SDK | 8/8 | Complete | 2026-09-13 |
| 4. Product Web | 9/9 | Complete | 2026-09-13 |
| 5. Agents and Evidence | 13/13 | Complete | 2026-09-13 |
| 6. Candle and Portfolio | 0/TBD | Not started | - |
