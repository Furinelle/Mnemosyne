"""Eval corpus schema and JSONL serialization."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass
class EvalItem:
    query: str
    expected_ids: list[str]
    paraphrase_of: str
    notes: str = ""


def load_corpus(path: Path) -> list[EvalItem]:
    items: list[EvalItem] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                items.append(EvalItem(**json.loads(line)))
    return items


def save_corpus(path: Path, items: list[EvalItem]) -> None:
    with path.open("w", encoding="utf-8") as handle:
        for item in items:
            handle.write(json.dumps(asdict(item), ensure_ascii=False) + "\n")
