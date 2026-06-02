"""Retrieval quality and latency metrics."""

from __future__ import annotations

import math


def recall_at_k(results: list[str], expected: list[str], k: int) -> float:
    if not expected:
        return 0.0
    return len(set(results[:k]) & set(expected)) / len(set(expected))


def mrr(results: list[str], expected: list[str]) -> float:
    targets = set(expected)
    for rank, result in enumerate(results, start=1):
        if result in targets:
            return 1.0 / rank
    return 0.0


def latency_percentiles(durations: list[float]) -> dict[str, float]:
    if not durations:
        return {"p50": 0.0, "p99": 0.0}
    ordered = sorted(durations)
    return {
        "p50": _percentile(ordered, 0.50),
        "p99": _percentile(ordered, 0.99),
    }


def _percentile(ordered: list[float], quantile: float) -> float:
    index = max(0, min(len(ordered) - 1, math.ceil(len(ordered) * quantile) - 1))
    return ordered[index]
