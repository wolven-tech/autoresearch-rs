# Roadmap: Autoresearch Rust

## Overview

Build safe experiment substrate first, then Git isolation, evaluators, product adapters, mutation agents, reports, Candle training, parity proof, and reusable portfolio manifests. First usable release ends after reporting.

## Phases

- [x] **Phase 1: Foundation** - Workspace, typed decisions, frozen config, journal, and CLI shell.
- [x] **Phase 2: Git Loop** - Isolated worktrees, protected paths, branches, locks, and recovery. (completed 2026-09-12)
- [ ] **Phase 3: Evaluator SDK** - Native traits, JSONL protocol, command, Cargo, and diff adapters.
- [ ] **Phase 4: Product Web** - Browser, Lighthouse, accessibility, SEO/GEO adapters and fixture.
- [ ] **Phase 5: Agents and Evidence** - Mutation commands, static report, redaction, and export.
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
**Plans**: TBD

### Phase 4: Product Web
**Goal**: Measure browser, accessibility, performance, SEO, GEO, and production properties without moving product gates.
**Depends on**: Phase 3
**Plans**: TBD

### Phase 5: Agents and Evidence
**Goal**: Drive provider-neutral mutations and produce auditable static reports and redacted bundles.
**Depends on**: Phase 4
**Plans**: TBD

### Phase 6: Candle and Portfolio
**Goal**: Add Candle training, parity fixtures, and reusable weekly-bet manifests after product engine is usable.
**Depends on**: Phase 5
**Plans**: TBD

## Progress

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation | 4/4 | Complete | 2026-09-11 |
| 2. Git Loop | 4/4 | Complete | 2026-09-12 |
| 3. Evaluator SDK | 0/TBD | Not started | - |
| 4. Product Web | 0/TBD | Not started | - |
| 5. Agents and Evidence | 0/TBD | Not started | - |
| 6. Candle and Portfolio | 0/TBD | Not started | - |
