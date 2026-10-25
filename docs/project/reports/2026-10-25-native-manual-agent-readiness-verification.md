# Native Allstack verification: manual-agent readiness

## Application outcome

The actual autoresearch CLI now accepts the exact lowercase `manual` agent declaration during doctor readiness checks. It preserves the `agent_executable` JSON field, validates nonmanual agents, and always validates evaluator executables. The [request](../plans/2026-10-25-manual-agent-readiness-request.md) and [accepted plan](../plans/2026-10-25-manual-agent-readiness-plan.md) define the scope.

## Installed toolkit

The installed `~/.cargo/bin/allstack` has SHA-256 `9d18314eff02b59ef78e12507db46c85f24821351070c71c935f8fc9f7b11d34`. It is the retained tested binary from Allstack PR #5. Its source tree was compared with merged Allstack main `7c6f5809af3f43937032a6fccaac809e2a51aac7` and matched the verified source at `1093bd345c22385ab64c0dee8c64172f66f8f51e`. Installation copied that verified executable atomically; it did not rebuild the toolkit or rely on its unchanged `0.1.0` version string.

`allstack skills` listed seven native methods. Skills-only installation created both host entry points and documentation without opening stores. A subsequent check reported all seventeen resources unchanged. Durable-delivery setup then initialized real Chronis and an explicit project verification policy.

## Authenticated native flow

| Stage | Actual execution | Evidence |
| --- | --- | --- |
| Plan | Claude `/allstack-plan`, session `3d2d362c-9c2e-4292-bb0a-a0d4a8b40061` | Native slash-command expansion and source-read tool results retained; successful bounded process receipt. |
| Build | Codex native planning/recall followed by implementation | Twenty-two completed commands, actual project `allstack recall` result, source-digest comparison, and one file-change event affecting exactly three assigned application files. |
| Review | Claude `/allstack-review`, session `771ebee0-0936-4190-99ca-88dfc07bac6c` | Actual source reads and native invocation retained; no actionable findings; source review explicitly did not claim runtime execution. |
| QA | Claude `/allstack-qa`, session `480b32ac-ca87-493d-a8b7-1394e7754035` | Six required application commands and their actual tool results retained. |
| Delivery | Installed Allstack's adoption, verification and publication protocol | The project policy binds gates to one commit; runtime evidence and PR validation carry the exact commit and destination readbacks. |

Both hosts used existing authenticated accounts, configured models, and normal permission modes. No model override or permission bypass was supplied. Claude QA attempted one additional `rtk ls` availability probe that its tool allowlist denied. All six required application commands executed. That denial is retained in the private receipt rather than described as a successful probe.

The project Prime decision node is `759c9ba5-6786-4a57-bb54-a40f8a1b3ef9`. Its source digest, `0d1e7099a6c3e4b23b60a8ab4cc73b2995dd8bbf9f55709d7ce8c878ceeb160d`, matched the accepted plan when Codex recalled it. Native planning, implementation and review preceded managed-run enrollment; an active delivery run was not required for those methods.

## Frozen regression evidence

The same eight process-level tests were run against unmodified current main `f4d03b69691ce8bc74839dc1e1132ca5666ed4a3` and the repaired source. Test bytes remained unchanged, with SHA-256 `17e489072c98543ee1a47f60645285e316f22d4cfe21f895b839608e3b114158`.

| Source | Executed | Passed | Failed |
| --- | ---: | ---: | ---: |
| Current main | 8 | 4 | 4 |
| Repaired candidate | 8 | 8 | 0 |

The failing baseline cases were manual JSON readiness, manual text readiness, manual-agent/missing-evaluator separation, and an evaluator literally named `manual`. Existing command-agent behavior, case sensitivity and nonexecutable rejection passed on both sides. The new suite also snapshots repository bytes, including private run state, and installs executable write traps to detect accidental program execution.

The candidate CLI binary used for native QA has SHA-256 `d646f430812d3234c1af22fda4c1ee3c54a2e0eaf252c16326eb67ab6a7abe23`. Its bytes and the reviewed application sources were checked again after QA. Scoped `cargo clippy --locked -p autoresearch-cli --all-targets -- -D warnings` passed.

## Actual CLI outcomes

All commands exercised the built `autoresearch` executable with isolated Git repositories and concrete manifests. The coordinator independently repeated the six cases with separate stdout/stderr capture and before/after fixture snapshots.

| Input | Exit | Observed result |
| --- | ---: | --- |
| Manual agent, valid evaluator, JSON | 0 | `ready: true`; `agent_executable` passes with `manual agent; no executable required`. |
| Missing command agent | 4 | `ready: false`; agent executable check fails. |
| Manual agent, missing evaluator | 4 | Agent check passes; evaluator executable check fails. |
| Valid command agent and evaluator | 0 | Both executables resolve; readiness passes. |
| Agent named `Manual` | 4 | Case-sensitive near-match receives ordinary executable validation and fails. |
| Manual agent, valid evaluator, text | 0 | Same successful verdict and check identifiers as JSON. |

Stderr was empty in every independent rerun, and fixture bytes remained unchanged. Claude's Bash results combine stream presentation, so stream separation is established by the independent capture rather than inferred from the model's report.

## Delivery boundary and enforcement

The indexed [project policy](../../../workflows/verification.toml) requires workspace formatting, all CLI tests, scoped CLI lint, and whitespace validation at the prepared commit. Tests run serially because existing CLI fixtures compile helper executables. Each policy command has a 270-second deadline. Private host and gate receipts remain under ignored `.allstack/` and are read through Allstack's evidence commands.

The repository permits rebase merges only. At inspection, GitHub returned no rulesets and reported `main` unprotected; configured local hooks contained only `commit-msg`. Therefore GitHub CI is advisory. Allstack's managed publication requires its own successful exact-commit verification; this report does not imply that a manual Git push or GitHub merge is globally fenced.

This proof covers sequential authenticated hosts on one machine, a real application's CLI behavior, local lexical project Prime, and verified publication to an open PR. It does not establish browser behavior, hosted memory synchronization, deployment, or merge. The installed global autoresearch executable was not replaced.
