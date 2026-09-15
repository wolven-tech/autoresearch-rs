# How To: Write an Evaluator

An evaluator is a standalone binary that runs after a candidate is committed. It reads one JSON request on stdin, measures the candidate commit, and writes exactly one JSON line to stdout. autoresearch-rs does not know what a bundle byte, an axe node, or a SEO score is. You write the evaluator.

## The protocol in 30 seconds

1. autoresearch-rs writes one JSON object to stdin, then closes it.
2. Your evaluator reads the request, runs tests, measures, and computes.
3. Your evaluator writes exactly one JSON line to stdout. This is your answer.
4. Your evaluator exits 0 (success) or non-zero (failure).

## Request format

The request is one JSON object with these fields:

```json
{
  "protocol_version": 1,
  "evaluator_id": "fixture",
  "run_id": "run-1",
  "baseline_commit": "0123456789abcdef0123456789abcdef01234567",
  "evaluated_commit": "0123456789abcdef0123456789abcdef01234567",
  "candidate_worktree": "/absolute/path/to/worktree",
  "changed_paths": ["site/index.html"],
  "declared_environment": {},
  "artifact_directory": "/absolute/path/to/artifacts",
  "cancellation_id": "cancel-1"
}
```

Key fields:

`protocol_version` is always 1.

`evaluator_id` is the name you declared in the manifest.

`run_id` is a stable experiment identifier; use this for logging.

`baseline_commit` and `evaluated_commit` are full Git commit hashes.

`candidate_worktree` is the directory containing the candidate's edits (your working directory when the evaluator runs).

`changed_paths` lists repository-relative paths that changed in the commit.

`artifact_directory` is a directory outside the worktree where you can store build output, keyed by commit hash so you can cache across evaluations.

`declared_environment` contains environment variables you declared in the manifest (currently always empty from the CLI).

## Environment

Evaluators run with an empty environment (no PATH, HOME, USER, LANG, TZ, or credentials) in the directory `candidate_worktree`. stdin closes after the request is written. stdout and stderr are each capped at 1 MiB.

Since the environment is empty, use absolute paths for every tool and file (for example, `/usr/bin/python3` instead of `python3`). Read credentials from the filesystem if needed; store them in a local `.env` file in the worktree if your evaluator requires them. Always clean up generated files from the worktree because gitignored output is checked after every evaluator, and untracked ignored files block candidate commits.

## Response format

Write exactly one JSON line to stdout:

### Success response

```json
{
  "protocol_version": 1,
  "result": {
    "status": "success",
    "output": {
      "evaluator_id": "fixture",
      "run_id": "run-1",
      "baseline_commit": "0123456789abcdef0123456789abcdef01234567",
      "evaluated_commit": "0123456789abcdef0123456789abcdef01234567",
      "measurements": [
        {
          "kind": "hard_gate",
          "name": "cta_present",
          "outcome": {"passed": true, "detail": null}
        },
        {
          "kind": "numeric",
          "name": "paragraph_count",
          "metric_kind": "objective",
          "direction": "minimize",
          "value": 1.0
        }
      ],
      "observations": [],
      "artifacts": [],
      "warnings": []
    }
  }
}
```

Echo back `evaluator_id`, `run_id`, `baseline_commit`, and `evaluated_commit` from the request.

Measurements must include every gate and metric you declared in the manifest, with the right kind and direction, and no others.

Each `hard_gate` must pass or the candidate is discarded immediately.

Each `numeric` metric must have `metric_kind` of `objective`, `tie_breaker`, or `diagnostic`, plus a `direction` (maximize or minimize).

Observations, warnings, and artifacts are optional:

`observations` is an array of objects with `code` and `detail` keys.

`warnings` is an array of objects with `code` and `detail` keys.

`artifacts` is an array of objects with `name`, `relative_path` (under `artifact_directory`), and `media_type` keys.

Exit 0.

### Failure response

If measurement fails (evaluator crashes, file not found, etc.):

```json
{
  "protocol_version": 1,
  "result": {
    "status": "failure",
    "failure": {
      "class": "reported",
      "detail": "could not find site/index.html"
    }
  }
}
```

Failure classes describe what went wrong:

| Class | Meaning |
|---|---|
| `reported` | You detected an error and reported it (evaluator exit 0) |
| `spawn` | CLI could not spawn the evaluator process |
| `non_zero_exit` | Evaluator exited non-zero without JSON output |
| `timeout` | Evaluator exceeded the timeout |
| `protocol` | Invalid JSON or missing fields |

Exit 0 or non-zero (CLI does not check).

## Designing your evaluator

Before writing an evaluator, decide what you want to measure and how. The best evaluators are fast, deterministic, and measure something that correlates with your product goal. Avoid measuring implementation details; measure user-visible outcomes instead. A bundle size correlates with performance because users see faster pages, but linting errors do not. A build time matters only if deployments are slow.

Write evaluators in the language that fits your measurement. Rust or Python work well for anything CPU-intensive, complex logic, or integration with build tools. Shell works well when you need to wrap an existing tool or measure file content quickly. Whatever language you choose, make sure your evaluator runs in well under its timeout; a 10-second timeout is reasonable for most cases, but increase it if you need to compile, run a test suite, or collect many samples.

The simplest evaluators read a file and measure it. More complex evaluators run a build or compilation step. The most demanding run full integration tests or benchmarks. Start simple: prove the measurement works before adding the complexity of a build system or a benchmarking framework.

## Example: paragraph counter (Rust)

This Rust evaluator implements the full protocol end-to-end. It reads the request from stdin, extracts the candidate worktree path, reads the HTML file, and counts paragraphs. It then validates the presence of a CTA element as a hard gate and reports both measurements back to autoresearch-rs.

Here is the complete evaluator from the first-loop tutorial:

```rust
use std::fs;
use std::io::{self, Read};

fn field(request: &str, key: &str) -> String {
    let prefix = format!("\"{key}\":\"");
    let value = request.split(&prefix).nth(1).unwrap_or("");
    value.split('"').next().unwrap_or("").to_owned()
}

fn main() {
    let mut request = String::new();
    io::stdin().read_to_string(&mut request).expect("read stdin");

    let evaluator_id = field(&request, "evaluator_id");
    let run_id = field(&request, "run_id");
    let baseline_commit = field(&request, "baseline_commit");
    let evaluated_commit = field(&request, "evaluated_commit");
    let candidate_worktree = field(&request, "candidate_worktree");

    let html_path = format!("{}/site/index.html", candidate_worktree);
    let html_content = match fs::read_to_string(&html_path) {
        Ok(content) => content,
        Err(_) => {
            let response = format!(
                "{{\"protocol_version\":1,\"result\":{{\"status\":\"failure\",\"failure\":{{\"class\":\"reported\",\"detail\":\"could not read {}\"}}}}}}",
                html_path
            );
            println!("{response}");
            return;
        }
    };

    let paragraph_count = html_content.matches("<p>").count() as i32;
    let cta_present = html_content.contains("class=\"cta\"");

    let cta_gate = if cta_present {
        "\"outcome\":{\"passed\":true,\"detail\":null}"
    } else {
        "\"outcome\":{\"passed\":false,\"detail\":\"CTA link missing\"}"
    };

    let response = format!(
        "{{\"protocol_version\":1,\"result\":{{\"status\":\"success\",\"output\":{{\"evaluator_id\":\"{}\",\"run_id\":\"{}\",\"baseline_commit\":\"{}\",\"evaluated_commit\":\"{}\",\"measurements\":[{{\"kind\":\"hard_gate\",\"name\":\"cta_present\",{}}},{{\"kind\":\"numeric\",\"name\":\"paragraph_count\",\"metric_kind\":\"objective\",\"direction\":\"minimize\",\"value\":{}}}],\"observations\":[],\"artifacts\":[],\"warnings\":[]}}}}}}",
        evaluator_id, run_id, baseline_commit, evaluated_commit, cta_gate, paragraph_count
    );
    println!("{response}");
}
```

The key steps in this evaluator are: read stdin with `read_to_string`, parse field values using a simple string split (or use a JSON library for robustness), read the HTML file from the worktree, measure it, and construct a JSON response with all required fields. The response must include every metric declared in your manifest, no more and no fewer. Each hard gate's `passed` field determines whether the candidate is kept.

Compile it with the 2021 edition:

```bash
rustc --edition 2021 -o evaluator evaluator.rs
```

Point your manifest at the compiled binary:

```toml
[evaluators.command]
program = "/absolute/path/to/evaluator"
timeout_seconds = 10
```

The timeout should be generous enough for your measurement to complete, including any builds, network calls, or sampling loops. If measurement is fast (under a second), 10 seconds is plenty; for benchmarks or heavy analysis, increase it as needed.

## Example: shell wrapper

For simpler evaluators or to integrate existing tools, you can write the evaluator as a shell script. The script parses the JSON request using grep and cut (no external JSON parser needed), then drives your measurement logic. Shell evaluators work well for lightweight checks and file-based analysis. The same paragraph counter from the Rust example, implemented in bash:

```bash
#!/bin/bash
set -e

# Read the request
request=$(cat)
candidate_worktree=$(echo "$request" | grep -o '"candidate_worktree":"[^"]*"' | cut -d'"' -f4)
evaluator_id=$(echo "$request" | grep -o '"evaluator_id":"[^"]*"' | cut -d'"' -f4)
run_id=$(echo "$request" | grep -o '"run_id":"[^"]*"' | cut -d'"' -f4)
baseline_commit=$(echo "$request" | grep -o '"baseline_commit":"[^"]*"' | cut -d'"' -f4)
evaluated_commit=$(echo "$request" | grep -o '"evaluated_commit":"[^"]*"' | cut -d'"' -f4)

cd "$candidate_worktree"

# Run your test
page_renders=true
if ! grep -q "class=\"cta\"" site/index.html; then
  page_renders=false
fi

paragraph_count=$(grep -o "<p>" site/index.html | wc -l)

# Write response
cat <<EOF
{"protocol_version":1,"result":{"status":"success","output":{"evaluator_id":"${evaluator_id}","run_id":"${run_id}","baseline_commit":"${baseline_commit}","evaluated_commit":"${evaluated_commit}","measurements":[{"kind":"hard_gate","name":"cta_present","outcome":{"passed":${page_renders},"detail":null}},{"kind":"numeric","name":"paragraph_count","metric_kind":"objective","direction":"minimize","value":${paragraph_count}}],"observations":[],"artifacts":[],"warnings":[]}}}
EOF
```

Save as `evaluator.sh`, make it executable, and point the manifest at it.

Shell evaluators are simpler to write and debug, especially for file-based measurements. The tradeoff is speed: shell loops are slower than compiled code, and JSON construction by string concatenation is error-prone. Use shell for evaluators that run in under a second or so; for anything heavier, consider Rust or Python. Shell shines when you're wrapping an existing tool (a compiler, a linter, a benchmarking suite) and just need to parse its output.

## Handling noise

If your evaluator measures something inherently noisy (like Lighthouse samples, runtime benchmarks, or network latency), the metric will jitter between runs. A candidate that looks good in one evaluation may measure differently on a re-run, making it impossible for autoresearch-rs to reliably compare candidates.

Three strategies work well for noisy metrics. First, run multiple independent samples and report the mean or median to reduce variance. A benchmark run five times and averaged is far more stable than a single sample. Second, replace the noisy measurement with a deterministic algorithm. Static analysis is more stable than runtime measurement; a linter check beats a performance test. Third, move the metric to `tie_breaker` or `diagnostic` kind so it influences the decision only when other metrics are equal.

When a metric jitters, `verify` reports `drifted` when the fresh run measures differently than the original selection. This is not a bug in autoresearch-rs; it means your evaluator is not stable enough for exact comparison. The re-measured values differ from the cached decision because the underlying measurement is noisy. Either stabilize the measurement or demote the metric to a tiebreaker so instability does not block progress.

## Artifacts and caching

Build output should live in `artifact_directory`, not in the worktree. This design serves three purposes. First, it enables caching: if two candidates share unchanged source files, their builds can reuse the same artifacts, keeping evaluation fast. Second, it keeps the worktree clean; gitignored output left behind blocks candidate commits. Third, it gives you a place to store logs, intermediate results, and debug information for inspection after the experiment finishes.

Organize artifacts by commit hash to take advantage of caching. When evaluating multiple candidates, commits that share dependency trees or source files can reuse build results. autoresearch-rs handles the directory structure; you just need to create the subdirectory inside the artifact root:

```rust
let commit = field(&request, "evaluated_commit");
let artifact_dir = format!("{}/by-commit/{}", artifact_directory, commit);
std::fs::create_dir_all(&artifact_dir)?;
// Write build output here
```

For verification runs, which build from scratch with no cache, use a separate `verify/` subdirectory to keep them distinct from normal evaluation. This makes it easy to audit what changed during verification without mixing in cached data from the selection phase.

## Testing your evaluator

Before you run a real experiment, test your evaluator with mock data. Creating a test request and a minimal worktree lets you verify the response format and debug parsing logic without committing to a full experiment. The test request should match the exact structure that autoresearch-rs will send, including all required fields. Run your evaluator in isolation and inspect the JSON output:

```bash
# Create a test request
cat > test-request.json <<'EOF'
{
  "protocol_version": 1,
  "evaluator_id": "fixture",
  "run_id": "run-1",
  "baseline_commit": "0123456789abcdef0123456789abcdef01234567",
  "evaluated_commit": "0123456789abcdef0123456789abcdef01234567",
  "candidate_worktree": "/tmp/test-worktree",
  "changed_paths": ["file.txt"],
  "declared_environment": {},
  "artifact_directory": "/tmp/test-artifacts",
  "cancellation_id": "cancel-1"
}
EOF

# Create a test worktree
mkdir -p /tmp/test-worktree /tmp/test-artifacts

# Run the evaluator
cat test-request.json | ./evaluator | jq .
```

Check that the response passes these validations:

- The response is valid JSON. If parsing fails, autoresearch-rs reports a protocol error.
- All required fields are present in the output: `evaluator_id`, `run_id`, `baseline_commit`, `evaluated_commit`, and `measurements`.
- Every declared metric in your manifest appears in the measurements array. A missing metric causes autoresearch-rs to reject the response.
- No extra metrics appear. Metrics must match the manifest exactly; undeclared measurements are ignored or cause an error depending on the strictness setting.

## Full protocol reference

See [evaluator-protocol.md](../reference/evaluator-protocol.md) for the complete protocol, including failure classes, observations, warnings, and artifacts.

## Deploying your evaluator

Once your evaluator is tested and working, integrate it into your experiment by updating the manifest to point at the binary you built. Then validate your complete setup.

Start by running `doctor` to check your manifest, gates, and metrics. This catches configuration errors early without running measurements.

Next, run `baseline` to establish the base commit. This freezes the trunk and evaluates it against your metrics, giving you the reference point autoresearch-rs will use to compare candidates. A successful baseline means your evaluator works end-to-end on real code.

Then iterate with `run --hypothesis "..."` to generate and evaluate candidates. Each hypothesis advances toward the goals your metrics define.

## Debugging and troubleshooting

Evaluators fail for many reasons: file not found, parsing error, timeout, out of memory. autoresearch-rs reports the failure class and your detail message, but debugging needs more information.

Write output to stderr during development. autoresearch-rs captures both stdout and stderr, and stderr lets you log intermediate values, debug state, and error messages without interfering with the JSON response on stdout. A log line showing what file you tried to read, what parsing failed, or what measurement you got is invaluable when debugging.

Test your evaluator thoroughly before running a real experiment. Create mock requests with realistic field values, feed them to your evaluator, and verify the output JSON is correct. Add assertions to your code: check file existence before reading, validate JSON before sending. Fail early and clearly rather than silently producing wrong measurements.

Watch out for state leaks between evaluations. If your evaluator creates temporary files or caches, clean them up at the end of each evaluation. The artifact directory is meant for cached builds; the worktree must be clean when you exit. Gitignored files left behind block candidate commits, so read your .gitignore and respect it.

Environment isolation is strict: no PATH, no HOME, no shell. If your evaluator needs to invoke a tool (a compiler, a test runner, git), use an absolute path and pass arguments explicitly. Avoid shell expansion (`$HOME`, `~`, wildcards). Read your manifest's `declared_environment` to get any configuration variables you declared.

If your evaluator measures something that is inherently noisy, be honest about it. Use `diagnostic` metrics for speculative measurements and report the uncertainty. autoresearch-rs filters candidates by gates first, then by objective metrics; diagnostic metrics show up in reports but do not drive selection. A measurement you are uncertain about belongs in the diagnostic tier.

## Evaluator patterns

Different products need different measurements. A content site measures readability and SEO signals. A web service measures API response time and error rates. A compiler measures output quality and build speed. The evaluator is the bridge between your product and autoresearch-rs; it translates domain-specific measurements into metrics.

Static analysis evaluators are fast, reproducible, and easy to debug. Read the source, run a parser or linter, count violations. No randomness, no environment dependencies. Use this pattern for code quality, style conformance, or structural checks.

Build-based evaluators run compilation, bundling, or a test suite. They measure output artifacts: bundle size, test coverage, compilation time. These evaluators are slower but can measure real product properties that static analysis cannot reach. Cache builds aggressively to keep evaluation speed reasonable.

Benchmark evaluators measure runtime behavior: throughput, latency, memory usage. These are the slowest and noisiest evaluators. Run multiple samples, report aggregates (mean or median), and gate on the mean to filter the jitter. Benchmarks are best used as tie-breakers once the candidate has passed structural gates.

Hybrid evaluators combine multiple measurements: a fast gate (static check), then heavier measurements only if the gate passes. This structure lets you reject obviously broken candidates quickly without burning time on expensive benchmarks.

Choose the lightest evaluator that measures what you care about. A static check beats a build, a build beats a benchmark, and a cached measurement beats a cold one.
