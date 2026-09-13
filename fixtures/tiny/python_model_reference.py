"""Opt-in dependency-free numeric reference for the tiny Candle shape.

This Python side is necessary for cross-language comparison. It is not
upstream nanochat, PyTorch, BPE, RoPE, FA3, or a full optimizer parity oracle.
"""

from __future__ import annotations

import json
import math
import struct
from pathlib import Path

from python_reference import emit as emit_data

MASK64 = (1 << 64) - 1
MULTIPLIER = 6364136223846793005


def f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


class Initializer:
    def __init__(self, seed: int) -> None:
        self.state = seed
        self.parameters: dict[str, list[list[float]] | list[float]] = {}

    def weight(self, name: str, rows: int, columns: int) -> list[list[float]]:
        result = []
        for _ in range(rows):
            row = []
            for _ in range(columns):
                self.state = (self.state * MULTIPLIER + 1) & MASK64
                raw = self.state >> 48
                value = f32(f32(f32(raw) / f32(65535)) - f32(0.5))
                row.append(f32(value * f32(0.1)))
            result.append(row)
        self.parameters[name] = result
        return result

    def norm(self, name: str, width: int) -> None:
        self.parameters[name + ".gamma"] = [1.0] * width
        self.parameters[name + ".beta"] = [0.0] * width


def initialize(seed: int) -> dict[str, list[list[float]] | list[float]]:
    init = Initializer(seed)
    init.weight("token_embedding", 257, 16)
    init.weight("position_embedding", 8, 16)
    init.norm("blocks.0.attention_norm", 16)
    for name in ("query", "key", "value", "projection"):
        init.weight("blocks.0." + name, 16, 16)
    init.norm("blocks.0.feed_forward_norm", 16)
    init.weight("blocks.0.feed_forward_in", 32, 16)
    init.weight("blocks.0.feed_forward_out", 16, 32)
    init.norm("final_norm", 16)
    init.weight("output", 257, 16)
    return init.parameters


def matrix(params: dict, name: str) -> list[list[float]]:
    value = params[name]
    assert isinstance(value, list) and value and isinstance(value[0], list)
    return value


def vector(params: dict, name: str) -> list[float]:
    value = params[name]
    assert isinstance(value, list) and value and isinstance(value[0], float)
    return value


def linear(x: list[float], weights: list[list[float]]) -> list[float]:
    return [sum(w * v for w, v in zip(row, x)) for row in weights]


def norm(x: list[float], params: dict, name: str) -> list[float]:
    mean = sum(x) / len(x)
    centred = [value - mean for value in x]
    variance = sum(value * value for value in centred) / len(x)
    scale = 1.0 / math.sqrt(variance + 1.0e-5)
    gamma = vector(params, name + ".gamma")
    beta = vector(params, name + ".beta")
    return [value * scale * g + b for value, g, b in zip(centred, gamma, beta)]


def softmax(values: list[float]) -> list[float]:
    peak = max(values)
    raised = [math.exp(value - peak) for value in values]
    total = sum(raised)
    return [value / total for value in raised]


def forward(
    params: dict, inputs: list[list[int]]
) -> list[list[list[float]]]:
    token = matrix(params, "token_embedding")
    position = matrix(params, "position_embedding")
    output: list[list[list[float]]] = []
    for row in inputs:
        hidden = [
            [a + b for a, b in zip(token[ident], position[index])]
            for index, ident in enumerate(row)
        ]
        prefix = "blocks.0."
        normal = [norm(value, params, prefix + "attention_norm") for value in hidden]
        queries = [linear(value, matrix(params, prefix + "query")) for value in normal]
        keys = [linear(value, matrix(params, prefix + "key")) for value in normal]
        values = [linear(value, matrix(params, prefix + "value")) for value in normal]
        attended = []
        for index in range(len(row)):
            channels = [0.0] * 16
            for head in range(2):
                offset = head * 8
                weights = softmax(
                    [
                        sum(
                            queries[index][offset + channel]
                            * keys[previous][offset + channel]
                            for channel in range(8)
                        ) / math.sqrt(8)
                        for previous in range(index + 1)
                    ]
                )
                for channel in range(8):
                    channels[offset + channel] = sum(
                        weight * values[previous][offset + channel]
                        for previous, weight in enumerate(weights)
                    )
            attended.append(channels)
        projected = [
            linear(value, matrix(params, prefix + "projection"))
            for value in attended
        ]
        residual = [
            [a + b for a, b in zip(left, right)]
            for left, right in zip(hidden, projected)
        ]
        feed = []
        for value in residual:
            normal_ff = norm(value, params, prefix + "feed_forward_norm")
            inner = linear(normal_ff, matrix(params, prefix + "feed_forward_in"))
            activated = [max(0.0, scalar) ** 2 for scalar in inner]
            feed.append(linear(activated, matrix(params, prefix + "feed_forward_out")))
        hidden = [
            [a + b for a, b in zip(left, right)]
            for left, right in zip(residual, feed)
        ]
        output.append(
            [
                linear(norm(value, params, "final_norm"), matrix(params, "output"))
                for value in hidden
            ]
        )
    return output


def loss(logits: list[list[list[float]]], targets: list[list[int]]) -> float:
    total = 0.0
    count = 0
    for row, target_row in zip(logits, targets):
        for values, target in zip(row, target_row):
            peak = max(values)
            total += peak + math.log(sum(math.exp(v - peak) for v in values)) - values[target]
            count += 1
    return total / count


def emit(root: Path) -> dict[str, object]:
    data = emit_data(root)
    params = initialize(int(data["seed"]))
    inputs = data["batch_inputs"]
    targets = data["batch_targets"]
    assert isinstance(inputs, list) and isinstance(targets, list)
    logits = forward(params, inputs)
    before_loss = loss(logits, targets)
    selected = matrix(params, "output")[0]
    before = selected[0]
    epsilon = 1.0e-3
    selected[0] = before + epsilon
    positive = loss(forward(params, inputs), targets)
    selected[0] = before - epsilon
    negative = loss(forward(params, inputs), targets)
    selected[0] = before
    gradient = (positive - negative) / (2.0 * epsilon)
    selected[0] = before - 0.01 * gradient
    after_selected_loss = loss(forward(params, inputs), targets)
    return {
        "schema_version": 1,
        "reference": "python_stdlib_tiny_model_v1",
        "seed": data["seed"],
        "device": "cpu",
        "dtype": "python_f64_with_f32_initial_weights",
        "batch_inputs": inputs,
        "batch_targets": targets,
        "batch_indices": data["batch_indices"],
        "train_token_ids": data["train_token_ids"],
        "validation_token_ids": data["validation_token_ids"],
        "logits": logits,
        "loss": before_loss,
        "selected_parameter": "output[0,0]",
        "selected_before": before,
        "selected_gradient": gradient,
        "selected_after_sgd": selected[0],
        "loss_after_selected_update": after_selected_loss,
        "full_optimizer_step": "not_implemented",
    }


if __name__ == "__main__":
    print(json.dumps(emit(Path(__file__).resolve().parent), sort_keys=True))
