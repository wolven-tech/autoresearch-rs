# Your first loop

In this lesson you run a complete experiment on a tiny landing page: you freeze a contract, measure a baseline, submit two candidates, and watch the runner keep one and discard the other. The second candidate scores better than the first and is still discarded, which is the behaviour the whole tool exists for.

Everything you need is in this repository. You will not design an evaluator or invent a metric here; both are provided, and each command's real output is shown so you can compare.

Allow about fifteen minutes.

## Before you start

You need Rust 1.97.1, the version pinned in [rust-toolchain.toml](../../rust-toolchain.toml), and `git` with `user.name` and `user.email` configured, because the runner makes commits as you.

Build the CLI from a checkout of this repository:

```bash
cargo install --path apps/autoresearch-cli
autoresearch --version
```

That prints the installed version, 0.1.0.

## Step 1: build the evaluator

The evaluator is the program that produces your number. This one counts `<p>` elements in a page and checks that a call-to-action link survives. Its source is [examples/first-loop/evaluator/main.rs](../../examples/first-loop/evaluator/main.rs), and it uses only the standard library.

Compile it to an absolute path, because evaluators run with an empty environment and cannot search your `PATH`:

```bash
mkdir -p /tmp/first-loop
rustc --edition=2024 -O examples/first-loop/evaluator/main.rs -o /tmp/first-loop/evaluator
```

## Step 2: make a product repository

Run the loop against a throwaway repository, not against this checkout. Copy the example page into it and initialise Git:

```bash
mkdir -p /tmp/first-loop/product
cp -R examples/first-loop/site /tmp/first-loop/product/site
git -C /tmp/first-loop/product init -q
```

The page has two paragraphs and one link with `class="cta"`.

## Step 3: create the contract

`init` writes a manifest and a brief, and never overwrites files that already exist:

```bash
autoresearch --repository /tmp/first-loop/product init
```

```text
repository: /tmp/first-loop/product
autoresearch.toml: created
program.md: created
```

The template it writes cannot run, because its evaluator is `git diff --numstat`, which does not speak the JSONL protocol. Replace both files with the example's, which declare the real objective and gate:

```bash
cp examples/first-loop/autoresearch.toml /tmp/first-loop/product/autoresearch.toml
cp examples/first-loop/program.md /tmp/first-loop/product/program.md
```

Now open `/tmp/first-loop/product/autoresearch.toml` and replace `/ABSOLUTE/PATH/TO/first-loop-evaluator` with `/tmp/first-loop/evaluator`. The manifest declares one objective, `paragraph_count`, to minimize, and one hard gate, `cta_present`.

## Step 4: freeze and measure the baseline

The run state lives in `.autoresearch/`, which must be ignored, and every frozen input must be committed before a baseline:

```bash
printf '.autoresearch/\n' > /tmp/first-loop/product/.gitignore
git -C /tmp/first-loop/product add .
git -C /tmp/first-loop/product commit -q -m "chore: freeze autoresearch contract"
autoresearch --repository /tmp/first-loop/product baseline
```

```text
run: run-1789484617268-87793-faadd3f3
base commit: faadd3f36635eb0bc453f5b6817b022d570d5534
frozen identity: 6f2c8da57e06d9055e13b6c56e6590ba617f705db830896b42afec51a1ea1ba4
environment fingerprint: ca474ad3c79641f58bdbe73cb56a9344bb81bb04deb9c0927ae4b489ebe0b208
evidence: captured
next: run
directory: /tmp/first-loop/product/.autoresearch/runs/run-1789484617268-87793-faadd3f3
snapshot: {"measurements":[{"kind":"hard_gate","name":"cta_present","outcome":{"passed":true,"detail":null}},{"kind":"numeric","name":"paragraph_count","metric_kind":"objective","direction":"minimize","value":2.0}],"complexity":{"changed_lines":0,"dependency_delta":0,"runtime_ms":0}}
```

Your run id and hashes will differ. The baseline scored `paragraph_count` 2 with the gate passing. Keep the run id to hand; every later command needs it.

## Step 5: open a candidate

```bash
autoresearch --repository /tmp/first-loop/product run --run-id RUN_ID
```

```text
run: run-1789484617268-87793-faadd3f3
status: awaiting_mutation
worktree: /tmp/first-loop/product/.autoresearch/worktrees/run-1789484617268-87793-faadd3f3/candidate-000001
```

The runner made an isolated worktree and is waiting. Your own checkout is untouched.

## Step 6: make one change and submit it

In that worktree, delete the second paragraph from `site/index.html`, the line reading `<p>It measures page content and finds the best balance.</p>`. Leave the edit uncommitted; the runner commits it for you. Then submit:

```bash
autoresearch --repository /tmp/first-loop/product --json run --run-id RUN_ID --hypothesis "one paragraph is enough to make the point"
```

The JSON carries the decision:

```json
{"status":"evaluated","commit":"a59fa309e87f74bad471acfbae1c6d616b0139fa","decision":{"disposition":"keep","reason":{"reason":"primary_improvement"}},"outcome":"kept"}
```

`paragraph_count` fell from 2 to 1 with the gate still passing, so the candidate was kept and the run branch moved to it.

## Step 7: submit a candidate that scores better and breaks the gate

Open a second candidate:

```bash
autoresearch --repository /tmp/first-loop/product run --run-id RUN_ID
```

In the new worktree, delete the remaining paragraph and the call-to-action line, leaving only the heading. Submit it:

```bash
autoresearch --repository /tmp/first-loop/product --json run --run-id RUN_ID --hypothesis "dropping the last paragraph scores better still"
```

```json
{"status":"evaluated","commit":"e9524bfcfd4080b7241a3f76cbd892af5f89865a","decision":{"disposition":"discard","reason":{"reason":"failed_hard_gates","names":["cta_present"]}},"outcome":"discarded"}
```

`paragraph_count` reached 0, better than the kept candidate's 1, and the candidate was discarded anyway. The objective was never compared, because a hard gate failed first and the reason names it.

## Step 8: verify and report

`verify` reruns the evaluators at the kept commit and compares every measurement:

```bash
autoresearch --repository /tmp/first-loop/product verify --run-id RUN_ID
```

```text
fresh verification: matched
```

`matched` means nothing drifted between the decision and now. Then read the run back:

```bash
autoresearch --repository /tmp/first-loop/product report --run-id RUN_ID
```

The report names the base commit, the current best commit, and one entry per candidate with its decision reason: `primary_improvement` for the first, `failed_hard_gates` for the second. The kept commit also sits on a local branch named after the run:

```bash
git -C /tmp/first-loop/product rev-parse autoresearch/RUN_ID
```

Nothing has touched your own branch. Merging that ref is a separate decision, described in [take a kept result](../how-to/take-a-kept-result.md).

## What this taught

A run is a contract plus a series of one-change candidates. The gate is read before the number, so a better score cannot buy a broken requirement, and every decision carries a reason you can read months later.

To run this on your own repository, write the evaluator that produces your number: [write an evaluator](../how-to/write-an-evaluator.md). To understand the decision order and the tie rules, read [how a run decides](../explanation/how-a-run-decides.md). Before quoting any number a run produces, read [what a keep does not prove](../explanation/limits.md).
