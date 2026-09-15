# Pinned upstream Python reference

Source: [karpathy/autoresearch](https://github.com/karpathy/autoresearch/tree/228791fb499afffb54b46200aca536f79142f117)
at commit `228791fb499afffb54b46200aca536f79142f117`. This commit is an
ancestor of the Rust fork; moving files here did not rewrite Git history.
Original README attributes the project to Andrej Karpathy and labels its
license “MIT”. That pinned tree has no standalone `LICENSE` file; this manifest
records the upstream statement, not a new legal conclusion or license grant.

`train.py` describes itself as “Cherry-picked and simplified from nanochat”.
[karpathy/nanochat](https://github.com/karpathy/nanochat) ships an MIT
`LICENSE` reading “Copyright (c) 2025 Andrej Karpathy”; that file is not part
of the pinned upstream tree and is not copied here. The repository root
`LICENSE` does not cover the ten files below, and the root `NOTICE` records
that boundary.

All ten upstream files below retain their exact Git blob IDs. The root Rust
README is new; `reference/python/README.md` is the original upstream README.

| Relative file | Pinned upstream Git blob |
| --- | --- |
| `.gitignore` | `99c30f52f1cb7b022668ec7215a604aa6b96f77a` |
| `.python-version` | `c8cfe3959183f8e9a50f83f54cd723f2dc9c252d` |
| `README.md` | `953ea55d5599c45c1be7dad93ec03e47dfa7df9d` |
| `analysis.ipynb` | `bef188375e292d8732a61bf1fa756a4614b76308` |
| `prepare.py` | `06bea9165abd3ae94ea82dd733997aec7928f40c` |
| `program.md` | `dea9bcc0174f1502d0ba64000b94b81ba605855b` |
| `progress.png` | `999e45c2b8833884870c1d7f084bab7a7bbc9e2c` |
| `pyproject.toml` | `94ae3298925ddae47821f703faf44d4e3b8381bb` |
| `train.py` | `2e743974c7f06b54311643b314712303fbb26e65` |
| `uv.lock` | `c840d62f5285b5ce4b5fe62f135cfe8a47bc9915` |

Run original commands from `reference/python`, so `uv` sees adjacent
`pyproject.toml`, `uv.lock`, and `.python-version`; `train.py` can import sibling
`prepare.py`, and notebook paths resolve beside `progress.png`. Upstream full
training requires its documented Python, data, and GPU environment. It is not
run as part of Rust's offline tests:

```bash
cd reference/python
uv sync
uv run prepare.py
uv run train.py
```

Check preserved bytes without executing Python:

```bash
git ls-tree -r 228791fb499afffb54b46200aca536f79142f117
git hash-object reference/python/prepare.py reference/python/train.py reference/python/README.md
git merge-base --is-ancestor 228791fb499afffb54b46200aca536f79142f117 HEAD
```
