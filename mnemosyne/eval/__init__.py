"""Deterministic retrieval evaluation harness."""

from __future__ import annotations

import json
import re
import time
from collections import defaultdict
from pathlib import Path
from typing import Callable

from mnemosyne.eval.corpus import EvalItem, load_corpus
from mnemosyne.eval.metrics import latency_percentiles, mrr, recall_at_k
from mnemosyne.search import BM25, SearchDocument
from mnemosyne.tokenizer import tokenize


def legacy_tokenize(text: str) -> list[str]:
    return [match.group(0).lower() for match in re.finditer(r"[\w\u4e00-\u9fff]+", text)]


def run_evaluation(
    corpus_path: Path,
    tokenizer: Callable[[str], list[str]] = tokenize,
) -> dict[str, float]:
    items = load_corpus(corpus_path)
    documents = _load_seed_documents(corpus_path.with_name("seed_memories.jsonl"))
    engine = BM25(documents, tokenizer=tokenizer)
    recalls: list[float] = []
    reciprocal_ranks: list[float] = []
    durations: list[float] = []
    for item in items:
        started = time.perf_counter()
        results = engine.search(item.query, limit=5)
        durations.append((time.perf_counter() - started) * 1000)
        result_ids = [result.document.id for result in results]
        recalls.append(recall_at_k(result_ids, item.expected_ids, 5))
        reciprocal_ranks.append(mrr(result_ids, item.expected_ids))
    latency = latency_percentiles(durations)
    return {
        "count": float(len(items)),
        "recall@5": _mean(recalls),
        "MRR": _mean(reciprocal_ranks),
        **latency,
    }


def format_metrics(backend: str, metrics: dict[str, float]) -> str:
    return (
        f"backend: {backend:<14} "
        f"recall@5={metrics['recall@5']:.3f}  "
        f"MRR={metrics['MRR']:.3f}  "
        f"p50={metrics['p50']:.3f}ms  "
        f"p99={metrics['p99']:.3f}ms  "
        f"queries={int(metrics['count'])}"
    )


def _load_seed_documents(path: Path) -> list[SearchDocument]:
    documents: list[SearchDocument] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            data = json.loads(line)
            documents.append(SearchDocument(id=str(data["id"]), text=str(data["text"])))
    return documents


def _mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def _load_seed_grouped(path: Path) -> dict[str, list[SearchDocument]]:
    """Group seed documents by instance_id (or a single '' bucket)."""
    grouped: dict[str, list[SearchDocument]] = defaultdict(list)
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            data = json.loads(line)
            grouped[str(data.get("instance_id", ""))].append(
                SearchDocument(id=str(data["id"]), text=str(data["text"]))
            )
    return grouped


def run_longmemeval(
    corpus_path: Path,
    tokenizer: Callable[[str], list[str]] = tokenize,
    ks: tuple[int, ...] = (1, 5, 10),
) -> dict:
    items = load_corpus(corpus_path)
    grouped = _load_seed_grouped(corpus_path.with_name("seed_memories.jsonl"))
    engines = {inst: BM25(docs, tokenizer=tokenizer) for inst, docs in grouped.items()}

    recalls: dict[int, list[float]] = {k: [] for k in ks}
    reciprocal_ranks: list[float] = []
    durations: list[float] = []
    by_type_recall: dict[str, dict[int, list[float]]] = defaultdict(
        lambda: {k: [] for k in ks}
    )

    for item in items:
        engine = engines.get(item.instance_id) or engines.get("")
        if engine is None:
            continue
        started = time.perf_counter()
        results = engine.search(item.query, limit=max(ks))
        durations.append((time.perf_counter() - started) * 1000)
        result_ids = [r.document.id for r in results]
        for k in ks:
            value = recall_at_k(result_ids, item.expected_ids, k)
            recalls[k].append(value)
            by_type_recall[item.question_type][k].append(value)
        reciprocal_ranks.append(mrr(result_ids, item.expected_ids))

    report: dict = {
        "count": float(len(items)),
        "MRR": _mean(reciprocal_ranks),
        **latency_percentiles(durations),
    }
    for k in ks:
        report[f"recall@{k}"] = _mean(recalls[k])
    report["by_type"] = {
        qtype: {f"recall@{k}": _mean(values[k]) for k in ks}
        for qtype, values in by_type_recall.items()
    }
    return report


__all__ = [
    "EvalItem",
    "format_metrics",
    "legacy_tokenize",
    "run_evaluation",
    "run_longmemeval",
]
