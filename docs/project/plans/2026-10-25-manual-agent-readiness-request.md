# Manual-agent readiness request

## Outcome

An operator using a valid manual-agent experiment can run `autoresearch doctor` and receive a ready result without installing an executable named `manual`. Actual command agents and every evaluator must still pass executable checks. Doctor remains read-only and must not execute declared programs.

## Existing behavior

The CLI's `doctor` function passes every agent declaration to executable resolution. The runner supports `RunMutationMode::Manual` independently of the agent program's executable path. The CLI reference documents the resulting false negative for `program = "manual"`.

An isolated repository with a valid manifest, nonblank program, ignored run state, and `/usr/bin/git` evaluator reproduced exit status 4. Its only failing check was `agent_executable`, with detail `` `manual` not found or not executable ``. This observation used the existing installed autoresearch binary; current-main regression tests must independently establish the source defect.

## Acceptance

- The exact manual sentinel passes the agent readiness check without executable lookup.
- Preserve the `agent_executable` check identifier and existing JSON report shape.
- A missing nonmanual agent still fails readiness with exit status 4.
- A missing evaluator still fails readiness when the agent is manual.
- A near-match such as `Manual` remains an ordinary executable declaration.
- Tests execute the real CLI boundary, include positive and negative outcomes, and leave source, manifest, program, and run state unchanged.
- Update the CLI reference to describe supported behavior.
- Authenticated Claude plans; Codex implements; Claude reviews and exercises the built CLI using installed native Allstack skills.
- Installed Allstack verifies the exact single commit, publishes the authorized feature branch and one PR, then reads back GitHub, Chronis, and project Prime state.

## Scope and authority

The user authorized `fix/manual-agent-readiness` and one PR in `wolven-tech/autoresearch-rs`. Changes include the focused repair, regression coverage, installed repository skills and delivery policy, and indexed plan and verification documents. Shipping ends at a verified commit and open PR. Merge, deployment, evaluator protocol changes, and new mutation modes are outside this request.

## Verification

Use the scoped `workflows/verification.toml`: workspace formatting, all `autoresearch-cli` tests, CLI lint including all targets, and whitespace validation. Each command has a 270-second ceiling. Run only one local compilation at a time. Retain private host receipts under ignored `.allstack/`; publish bounded findings and source provenance in the indexed verification report.
