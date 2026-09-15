# How To: Hand a Loop to an Agent

autoresearch-rs can be driven by an agent (Claude Code, Codex, or another LLM-based tool) to automatically propose and evaluate candidates. You define the contract and evaluator; the agent edits files, submits candidates, and writes up results.

Create your repository contract with `autoresearch init`, then run experiments:

```bash
autoresearch --repository /path/to/product init
autoresearch --repository /path/to/product baseline
autoresearch --repository /path/to/product run --run-id <id> --hypothesis "proposal"
autoresearch --repository /path/to/product verify --run-id <id>
autoresearch --repository /path/to/product report --run-id <id>
```

The init command creates `autoresearch.toml` and `program.md`. Baseline freezes the repository state and runs all evaluators at HEAD. Each run advances one candidate, and verify reruns evaluators at a finalized candidate to confirm measurements.

## Manual mode (simplest)

Point the agent at the candidate worktree opened by `run`, let it edit only files under `mutable_paths`, and submit with `--hypothesis`:

```bash
autoresearch --repository /path/to/product run --run-id <id>
```

Tell the agent: "The worktree is at `.autoresearch/worktrees/<run-id>/candidate-NNNNNN`. Edit `site/index.html` to improve it."

The agent edits the file without committing. Then you submit:

```bash
autoresearch --repository /path/to/product run --run-id <id> --hypothesis "agent's proposed change"
```

The `--hypothesis` is required to submit. Manual mode offers full visibility: the agent operates through standard tools (filesystem, version control) and can see exactly what changed; the `--hypothesis` text itself does not persist to the report, but the evaluation results do.

### Setup for Claude Code

1. Point Claude Code at the autoresearch-rs repository.
2. Have it read the skill in `skills/autoresearch/SKILL.md`.
3. Link the skill into your skills folder:
   ```bash
   ln -s /path/to/autoresearch-rs/skills/autoresearch ~/.claude/skills/autoresearch
   ```
4. Invoke the `/autoresearch` skill from Claude Code.

The skill knows:
- The CLI commands (baseline, run, verify, report).
- How to edit files in the worktree.
- How to read the manifest and program.md.
- How to format a result write-up.

The skill does not know:
- What to improve (that's your domain judgement).
- Whether the improvement is correct (the evaluator answers that).
- The reasoning (you supply the brief in program.md).

## Command mode (automated)

Command mode runs an agent inside the same call, with no human interaction between `run` invocations:

```bash
autoresearch --repository /path/to/product run --run-id <id> \
  --mode command \
  --hypothesis "automated proposal" \
  --allow-executable /opt/agents/my-agent
```

The agent receives:

```json
{
  "run_id": "run-...",
  "baseline_commit": "...",
  "evaluated_commit": "...",
  "candidate_worktree": "/path/to/worktree",
  "mutable_paths": ["site/"],
  "protected_paths": [],
  "objective": {
    "name": "paragraph_count",
    "direction": "minimize"
  },
  "hard_gates": ["cta_present"],
  "policy": "edit_files_only"
}
```

on stdin as one JSON object. The agent reads from stdin and exits 0 on success.

The agent:
- Edits files under `mutable_paths`.
- Does NOT commit (the CLI commits).
- Does NOT run evaluators (the CLI runs them).
- Exits 0 on success, non-zero on error.
- Has its stdout and stderr discarded (only byte counts are recorded).

### Requirements for command mode

- `agent.program` in the manifest must be an absolute path (e.g., `/opt/agents/my-agent`).
- `--allow-executable` must canonicalize to the same file as `agent.program`.
- The agent must exit 0 (non-zero exit is treated as a failure).
- `authority.allow` must be empty (authority declarations in the manifest do not grant access in the CLI).
- The agent has no PATH, no HOME, and no environment variables.

### Writing a command-mode agent (Python example)

```python
#!/usr/bin/env python3
import json
import sys
import os

request = json.loads(sys.stdin.read())
worktree = request["candidate_worktree"]
mutable = request["mutable_paths"]
objective = request["objective"]

os.chdir(worktree)

# Edit files under mutable_paths
html_path = os.path.join(mutable[0], "index.html")
with open(html_path) as f:
    content = f.read()

# Remove one paragraph if there are more than one
if content.count("<p>") > 1:
    lines = content.split("\n")
    filtered = [line for line in lines if not line.strip().startswith("<p>")]
    with open(html_path, "w") as f:
        f.write("\n".join(filtered))

sys.exit(0)
```

Make it executable:

```bash
chmod +x /opt/agents/my-agent
```

Point the manifest at it:

```toml
[agent]
program = "/opt/agents/my-agent"
timeout_seconds = 30
```

Then invoke:

```bash
autoresearch --repository /path/to/product run --run-id <id> \
  --mode command \
  --hypothesis "automated proposal" \
  --allow-executable /opt/agents/my-agent
```

### Limitations of command mode

- The agent cannot read the manifest or program.md (not in the request JSON).
- The agent cannot use external tools: no PATH, no network access (authority declarations in the manifest are no-ops).
- Stdout and stderr are discarded; only byte counts are recorded.
- No feedback loop: the agent does not see evaluation results and cannot adapt strategy.

For these reasons, manual mode is usually better. An agent that can read the manifest, see evaluator output, and adapt its strategy is more effective than one operating blind.

## Iterative mode: agent reads results

Combine the best of both:

1. Run baseline:
   ```bash
   autoresearch --repository /path/to/product baseline
   ```

2. Loop:
   ```bash
   for CANDIDATE in {1..5}; do
     # Open candidate
     autoresearch --repository /path/to/product run --run-id $RUN_ID
     
     # Agent edits the worktree
     # (e.g., via Claude Code, with the skill driving it)
     
     # Submit
     autoresearch --repository /path/to/product run --run-id $RUN_ID --hypothesis "proposal $CANDIDATE" --json > result.json
     
     # Agent reads result.json and decides what to try next
   done
   ```

This gives the agent full visibility: it can read the program.md, see the baseline, read evaluation results, and adapt its strategy for the next candidate.

## Integration with CI/CD

autoresearch-rs can run in CI as part of a workflow. Start with baseline to create the run:

```yaml
- name: Run autoresearch loop
  run: |
    autoresearch --repository . baseline
    
    # Find the most recent run ID
    RUN_ID=$(ls -t .autoresearch/runs | head -1)
    
    for i in {1..3}; do
      autoresearch --repository . run --run-id $RUN_ID
      # ... agent edits ...
      autoresearch --repository . run --run-id $RUN_ID --hypothesis "candidate $i"
    done
    
    autoresearch --repository . report --run-id $RUN_ID --json > report.json
    
- name: Upload report
  uses: actions/upload-artifact@v3
  with:
    name: autoresearch-report
    path: report.json
```

The baseline command freezes the clean repository state and creates a run directory. Each `run` call advances one candidate step.

For detailed integration steps, read the [Claude skill](../../skills/autoresearch/SKILL.md). The [first-loop tutorial](../tutorials/first-loop.md) walks through the complete command sequence, and [decision-policy.md](../reference/decision-policy.md) explains how candidates are kept or discarded.
