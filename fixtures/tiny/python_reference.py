"""Opt-in, stdlib-only Python emitter for frozen tiny byte-level fixture.

This is a test-specific reference, not upstream BPE packing or CUDA training.
Python is needed here to produce cross-language parity evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

MASK64 = (1 << 64) - 1
LCG_MULTIPLIER = 6364136223846793005


def load_documents(root: Path, name: str, expected_digest: str) -> list[str]:
    data = (root / name).read_bytes()
    if hashlib.sha256(data).hexdigest() != expected_digest:
        raise ValueError(f"frozen {name} SHA-256 mismatch")
    documents = data.decode("utf-8").splitlines()
    if not documents or any(not document for document in documents):
        raise ValueError(f"frozen {name} must contain nonblank documents")
    return documents


def encode(document: str, bos_token_id: int) -> list[int]:
    return [bos_token_id, *document.encode("utf-8")]


def shuffled_indices(count: int, seed: int) -> list[int]:
    order = list(range(count))
    state = seed
    for index in range(count - 1, 0, -1):
        state = (state * LCG_MULTIPLIER + 1) & MASK64
        other = state % (index + 1)
        order[index], order[other] = order[other], order[index]
    return order


def emit(root: Path) -> dict[str, object]:
    contract_bytes = (root / "contract.json").read_bytes()
    contract = json.loads(contract_bytes)
    if (
        contract["schema_version"] != 1
        or contract["tokenizer"] != "utf8_bytes_bos_v1"
        or contract["normalization"] != "none"
        or contract["bos_token_id"] != 256
        or contract["vocab_size"] != 257
    ):
        raise ValueError("unsupported tiny fixture contract")
    train_documents = load_documents(root, "train.txt", contract["train_sha256"])
    validation_documents = load_documents(
        root, "validation.txt", contract["validation_sha256"]
    )
    sequence_length = contract["sequence_length"]
    batch_size = contract["batch_size"]
    if not 1 <= batch_size <= len(train_documents) or sequence_length < 1:
        raise ValueError("invalid tiny fixture dimensions")
    train_tokens = [encode(doc, contract["bos_token_id"]) for doc in train_documents]
    validation_tokens = [
        encode(doc, contract["bos_token_id"]) for doc in validation_documents
    ]
    if any(len(row) < sequence_length + 1 for row in train_tokens + validation_tokens):
        raise ValueError("tiny document cannot fill fixed sequence")
    order = shuffled_indices(len(train_tokens), contract["seed"])
    rows = [train_tokens[index][: sequence_length + 1] for index in order[:batch_size]]
    validation_row = validation_tokens[0][: sequence_length + 1]
    return {
        "schema_version": 1,
        "contract_sha256": hashlib.sha256(contract_bytes).hexdigest(),
        "tokenizer": contract["tokenizer"],
        "seed": contract["seed"],
        "sequence_length": sequence_length,
        "batch_size": batch_size,
        "train_token_ids": train_tokens,
        "validation_token_ids": validation_tokens,
        "batch_indices": order[:batch_size],
        "batch_inputs": [row[:-1] for row in rows],
        "batch_targets": [row[1:] for row in rows],
        "batch_masks": [[1] * sequence_length for _ in rows],
        "validation_inputs": validation_row[:-1],
        "validation_targets": validation_row[1:],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fixture-root", type=Path, default=Path(__file__).resolve().parent
    )
    args = parser.parse_args()
    print(json.dumps(emit(args.fixture_root), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
