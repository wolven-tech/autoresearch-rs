# Autoresearch Rust

**One-liner**: Rust-native, domain-agnostic experiment runner for safely improving repositories against frozen evidence gates.

## Problem

Karpathy's loop proves a powerful pattern but binds it to one Python training workload. Product repositories need the same bounded mutate-measure-keep loop without letting agents rewrite evaluators, confuse internal scores with market evidence, or perform external actions.

## Success Criteria

- [ ] CLI can initialise, diagnose, baseline, run, resume, inspect, verify, report, and export a bounded experiment.
- [ ] Every candidate runs in an isolated Git worktree and can only change declared mutable paths.
- [ ] Frozen gates and primary objective decide candidates deterministically with append-only recovery evidence.
- [ ] Static HTML and JSON reports distinguish internal capability from market evidence.
- [ ] Product-web and Candle training adapters pass end-to-end fixtures.

## Constraints

- Rust workspace with workspace-enforced Clippy, rustdoc, formatting, tests, and `unsafe_code = "deny"`.
- Agent-agnostic commands; no embedded vendor SDKs or secrets.
- External actions denied unless explicitly authorised per run.
- Preserve upstream fork history, attribution, reference implementation, and existing architecture.
- No unbounded loop, dynamic plugin ABI, or promotion from clicks, traffic, impressions, praise, or synthetic telemetry.

## Out of Scope

- Distributed orchestration and automatic production deployment or outreach.
- Exact H100 performance parity before deterministic semantic parity passes.
