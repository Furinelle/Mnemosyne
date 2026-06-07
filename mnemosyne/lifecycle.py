"""Memory lifecycle operations."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import date
from pathlib import Path

from mnemosyne.schema import Memory
from mnemosyne.store import Store, move_to_archive, write_memory


@dataclass
class MaintainSummary:
    processed: int = 0
    decayed: int = 0
    deprecated: int = 0
    archived: int = 0
    core_candidates: list[Memory] = field(default_factory=list)


def decay(memory: Memory, amount: int) -> None:
    memory.strength -= amount


def promote(memory: Memory, amount: int) -> None:
    memory.strength += amount


def demote(memory: Memory, amount: int) -> None:
    memory.strength -= amount


def consolidate(memory: Memory, summary: str) -> None:
    memory.injection_summary = summary
    memory.canonical_summary = summary


def maintain_memory(
    store: Store,
    path: Path,
    memory: Memory,
    thresholds: dict,
    dry_run: bool = False,
) -> tuple[str, Memory | None]:
    decay_per_run = int(thresholds.get("decay_per_run", 1))
    deprecated_strength = int(thresholds.get("deprecated_strength", 5))
    archive_strength = int(thresholds.get("archive_strength", 30))
    core_strength = int(thresholds.get("core_strength", 80))
    core_access_count = int(thresholds.get("core_access_count", 3))

    # Enforce the expires field: a memory past its expiry date is archived
    # immediately, regardless of strength. Previously --expires was stored but
    # never read, so expired memories were injected forever.
    if memory.expires and memory.expires < date.today().isoformat():
        if not dry_run:
            yyyy_mm = (memory.last_accessed or memory.created or date.today().isoformat())[:7]
            write_memory(path, memory)
            move_to_archive(store, path, memory, yyyy_mm)
        return "archived", None

    decay(memory, decay_per_run)

    if memory.strength < deprecated_strength:
        memory.status = "deprecated"

    if memory.strength < archive_strength:
        if not dry_run:
            yyyy_mm = (memory.last_accessed or memory.created or date.today().isoformat())[:7]
            write_memory(path, memory)
            move_to_archive(store, path, memory, yyyy_mm)
        return "archived", None

    if memory.strength >= core_strength and memory.access_count >= core_access_count:
        if not dry_run:
            write_memory(path, memory)
        return "core_candidate", memory

    if not dry_run:
        write_memory(path, memory)
    if memory.status == "deprecated":
        return "deprecated", memory
    return "decayed", memory
