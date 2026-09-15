# Manifest Reference

The experiment contract lives in `autoresearch.toml` at the repository root. It specifies what you measure, the budget, the scope, the evaluators, and the gates.

## Schema

```toml
schema_version = 1

[experiment]
name = "your experiment name"

[experiment.objective]
name = "metric_name"
direction = "minimize"  # or "maximize"

[experiment.budget]
max_candidates = 10
max_failures = 3
wall_clock_seconds = 3600

[scope]
mutable_paths = ["path/to/mutable"]
protected_paths = ["path/to/protected"]

[agent]
program = "manual"  # or "/absolute/path/to/agent"
args = ["arg1", "arg2"]
timeout_seconds = 600

[[evaluators]]
id = "evaluator_name"
hard_gates = ["gate_name_1", "gate_name_2"]

[evaluators.command]
program = "/absolute/path/to/evaluator"
args = []
timeout_seconds = 120

[[evaluators.metrics]]
name = "metric_name"
kind = "objective"  # or "tie_breaker", "diagnostic", "market_evidence"
direction = "minimize"  # or "maximize"

[authority]
allow = []  # or ["network"] if evaluator needs network access
```

## Required sections

### schema_version

Must be `1`. Used for future format migrations.

### experiment

The experiment identity.

experiment.name sets the display name. It can be any string and is used in reports and logs.

experiment.objective specifies the metric that drives the decision.
- `name`: The objective metric name. Must match one of the metrics you declare in `[[evaluators.metrics]]` with `kind = "objective"`.
- `direction`: Either `minimize` or `maximize`. Determines whether higher or lower values are better.

### experiment.budget

Limits on the run.

| Field | Meaning |
|-------|---------|
| `max_candidates` | Maximum number of candidates to evaluate. The run stops with `candidate_limit` after this many. |
| `max_failures` | Maximum number of evaluator failures before the run stops. Currently validated but ignored; the run does not stop on failures through the CLI. |
| `wall_clock_seconds` | Maximum elapsed time for the run. Currently validated but ignored; use an external timer if you need to stop after a time. |

### scope

What candidates can change.

- `mutable_paths`: Array of paths that candidates can edit. Relative to the repository root. Required; at least one path must be specified.
- `protected_paths`: Array of paths that candidates cannot touch, even if under a mutable path. Optional. Useful for preventing candidates from changing a corpus or configuration.

A candidate that edits anything outside these paths, touches a symlink, a submodule, or a binary file results in an environment error (exit 4).

### agent

How the agent is invoked when not in manual mode.

| Field | Meaning |
|-------|---------|
| `program` | Either `manual` (no agent; you edit by hand) or an absolute path to a binary. |
| `args` | Array of string arguments to pass to the agent. Optional. Default is empty. |
| `timeout_seconds` | Maximum wall-clock time for the agent. Required; must be greater than 0. |

### evaluators

Array of `[[evaluators]]` sections. Each evaluator is a separate JSONL binary that runs after the candidate is committed.

evaluators.id is a name for this evaluator, used in reports and logs.

evaluators.hard_gates is an array of gate names that must pass. If any gate fails, the candidate is discarded immediately without comparing the objective. Each gate name must be a unique identifier across all evaluators and gates.

evaluators.requires_network is a boolean flag indicating if the evaluator requires network access. Optional; default is false. If true, requires `network` in the authority.allow list.

evaluators.command specifies how to run this evaluator.
- `program`: Absolute path to the evaluator binary.
- `args`: Optional array of command-line arguments. The evaluator never sees these in `argv` under the CLI (see [evaluator-protocol.md](evaluator-protocol.md)); they are reserved for future use.
- `timeout_seconds`: Maximum wall-clock time for the evaluator. Required; must be greater than 0. The CLI kills evaluators that exceed this timeout and reports a failure.

evaluators.metrics is an array of metrics this evaluator reports. Each `[[evaluators.metrics]]` entry declares a metric the evaluator will measure.

### evaluators.metrics

A metric that the evaluator reports.

- `name`: The metric name. Must match what the evaluator emits in its JSON output.
- `kind`: One of the metric kinds listed below.
- `direction`: For numeric metrics, either `minimize` or `maximize`. Optional; default is `minimize`.

| Kind | Purpose |
|------|---------|
| `objective` | The metric used for the decision. Only one per run. Better values are kept, worse values are discarded. |
| `tie_breaker` | Recorded but never used by selection. Ties on the objective are broken by the runner's measured complexity: changed lines, then dependency delta, then runtime milliseconds. |
| `diagnostic` | Recorded but never used by selection. Useful for context about the evaluation. |
| `market_evidence` | Commercial evidence kept separate from capability metrics. Recorded but never used by selection. |

Only one metric may have `kind = "objective"`. Only the objective drives the decision.

### authority

Permissions and network declarations.

- `allow`: Array of capabilities. Currently only `network` is recognized, and it has no effect; it is a declaration for future use. If your evaluator needs network access, keep the run in an isolated environment.

## Optional sections

The sections below can be omitted. If present, they override defaults.

## Example: page-weight loop

```toml
schema_version = 1

[experiment]
name = "landing page weight"

[experiment.objective]
name = "page_bytes"
direction = "minimize"

[experiment.budget]
max_candidates = 10
max_failures = 3
wall_clock_seconds = 3600

[scope]
mutable_paths = ["site"]
protected_paths = ["site/fixtures"]

[agent]
program = "manual"
timeout_seconds = 600

[[evaluators]]
id = "page_weight"
hard_gates = ["page_renders", "cta_present"]

[evaluators.command]
program = "/opt/autoresearch/bin/page-weight-evaluator"
timeout_seconds = 120

[[evaluators.metrics]]
name = "page_bytes"
kind = "objective"
direction = "minimize"

[authority]
allow = []
```

## Validation rules

The CLI validates the manifest on every command:

- `schema_version` must be `1`.
- `experiment.name` must be non-empty.
- `experiment.objective.name` must be declared as a metric with `kind = "objective"`, and exactly one metric must have that kind.
- `experiment.objective.direction` must be `minimize` or `maximize`.
- `experiment.budget.max_candidates` must be greater than 0.
- `scope.mutable_paths` must include at least one path.
- `agent.program` must be `manual` or an absolute path.
- Every hard gate name must be unique across all gates and metrics of all evaluators.
- Every metric name must be unique.
- At least one evaluator must be declared.

If any check fails, the command exits 3 with a description of the error.

## Frozen at baseline

The manifest is frozen when you run `baseline`. It is protected from candidates: a candidate that changes `autoresearch.toml` is rejected with an environment error. You can read the frozen manifest from the run directory.

## Initialization

Run `autoresearch init --repository /path/to/product` to create a template manifest and brief. The template has a placeholder evaluator and manual agent, so it will not run until you replace the evaluator path and configure the objective and scope.
