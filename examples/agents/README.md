# Mutation command examples

These are `[agent]` fragments for `autoresearch.toml`, not standalone
manifests. Keep the rest of the validated manifest, evaluator definitions, and
original `docs/BET.md` unchanged. Replace executable paths with absolute,
operator-reviewed paths. The runner accepts only executables supplied in its
separate per-run allowlist; manifest text alone never authorizes one.

Each command receives one bounded JSON `MutationRequest` on stdin and runs with
candidate worktree as cwd. It receives no inherited environment, HOME,
credential variables, or raw previous logs. The adapter captures only output
byte counts, enforces independent caps and timeout, and passes any edits through
Git containment before evaluation. Commands must not change manifest, program,
product gate, runner, or other protected files.

Codex and Claude examples illustrate identical local process contract, not
permission to call model services. Their normal remote inference requires
network and authentication. This adapter does not provide OS-level network or
descendant-process isolation; do not allowlist a network-capable or untrusted
binary until a separate trusted sandbox and explicit run authority exist. No
example requests deployment, outreach, purchase, payment, or production-write
capability. Manual mode remains available without a provider CLI.
