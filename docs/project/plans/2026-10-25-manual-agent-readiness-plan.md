# Manual-agent readiness plan

## Accepted outcome

Follow the [request](2026-10-25-manual-agent-readiness-request.md). The user authorized `fix/manual-agent-readiness` and one PR in `wolven-tech/autoresearch-rs`. Confidence: rock. The failing readiness check, manual runner path, and compatible reporting seam are identified.

## Current path and invariant

`apps/autoresearch-cli/src/lib.rs::doctor` validates the manifest and then unconditionally resolves the agent program through `check_executable`. An exact `manual` declaration therefore fails `agent_executable`, making `ready` false and returning exit status 4. The runner selects `RunMutationMode::Manual` from the CLI and never resolves an agent executable in that path.

Doctor must distinguish the supported manual declaration while retaining executable validation for every other agent declaration and every evaluator. It must remain read-only and must not run declared programs.

## Design

At the existing agent check in `doctor`, compare the program with the exact lowercase sentinel `manual`. Emit the existing `agent_executable` check with `passed = true` and an explanatory detail for that case. Keep `check_executable`, path resolution, evaluator iteration, JSON structure, and exit-code mapping intact.

A typed manifest mode would change configuration and frozen identity across crates. The doctor reporting seam expresses this requirement without those changes. No manifest version, dependency, execution mode, or runner behavior changes are needed.

## Files and sequence

1. Add external process-level regression coverage in `apps/autoresearch-cli/tests/doctor.rs`, using the actual CLI binary and isolated Git repositories. Avoid expanding the existing inline test module or the unrelated large CLI integration file.
2. Implement the agent-only condition in `apps/autoresearch-cli/src/lib.rs`.
3. Update the doctor section in `docs/reference/cli.md` to describe the manual exception and retain applicable evaluator caveats.
4. Run the frozen regression tests against current main to demonstrate failure, then against the candidate to demonstrate the repair. Keep their bytes unchanged across both runs.
5. Review and exercise the actual built CLI with Claude's installed `allstack-review` and `allstack-qa` methods. Inspect actual calls and independently repeat representative results.
6. Record bounded evidence under indexed project docs. Enroll the accepted candidate with `allstack adopt`; verify and publish its exact single commit through the authorized PR.

## Executable acceptance

- Exact manual agent with a resolvable evaluator: `doctor --json` exits 0, `ready` is true, and `agent_executable` remains present and passes.
- Missing nonmanual agent: exit 4, `ready` false, failed `agent_executable`.
- Manual agent with a missing evaluator: exit 4, passed agent check, failed evaluator check.
- Case-sensitive near-match `Manual`: executable resolution still applies.
- Check the text output and a valid ordinary command-agent case as compatibility coverage.
- Verify doctor leaves manifest, program, source and private run state unchanged and never executes declared programs.

The frozen project policy executes workspace formatting, all CLI tests, scoped CLI lint including all targets, and whitespace validation. A 270-second per-command deadline bounds every gate. Builds execute sequentially after competing compiler processes finish. Compilation alone cannot establish readiness behavior.

## Failure handling and publication

Invalid manifests, missing files, nonignored run state and missing evaluators retain their existing failed checks. The new condition is reached only for a parsed manifest and changes one readiness check. There are no new writes, subprocesses, retries, or partial-state recovery paths.

AllSource retains delivery history and receipts, Chronis owns claims and completion, and project Prime retains source-linked decisions. Native planning, implementation and review use host tools before enrollment. Publication requires positive tests, exact commit binding, branch and PR head readback, and confirmed task and memory outcomes. Shipping means an open PR; merge and deployment are outside scope.
