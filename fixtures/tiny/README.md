# Frozen tiny training fixture

`contract.json` pins UTF-8 byte tokens `0..255`, BOS `256`, no Unicode
normalization, seed `23`, sequence length `8`, batch size `2`, and SHA-256 of
both checked-in corpus splits. Each nonblank line is one document; line
separators are not tokens. Each encoded document starts with BOS. Batch order
uses a specified wrapping 64-bit LCG (`state = state * 6364136223846793005 +
1`) in reverse Fisher–Yates; first two shuffled documents provide first `T+1`
tokens. Inputs and targets are shifted by one. Every token is present, so masks
are all ones. Validation uses first validation document.

`python_reference.py` uses only Python standard library and emits
`golden-v1.json`; Python is needed for cross-language fixture evidence. Rust
default tests compare against that golden twice and reject corpus hash drift
without invoking Python, GPU, or network. To regenerate or inspect Python
output, run `python3 fixtures/tiny/python_reference.py` from repository root.

This is a controlled tiny byte-level contract. Upstream Python uses trained
rustbpe/tiktoken, BOS-aligned best-fit packing, CUDA/FA3, and a different
training schedule. Matching this fixture does not establish full upstream
semantic parity or H100 performance parity.
