"""Typed relation semantics for linked memories."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class RelationSpec:
    reverse: str
    weight: float
    symmetric: bool
    demote_target: bool = False
    warn: bool = False


PREDEFINED = {
    "caused_by": RelationSpec(reverse="causes", weight=0.6, symmetric=False),
    "refines": RelationSpec(reverse="refined_by", weight=0.7, symmetric=False),
    "supersedes": RelationSpec(
        reverse="superseded_by",
        weight=0.3,
        symmetric=False,
        demote_target=True,
    ),
    "contradicts": RelationSpec(
        reverse="contradicts",
        weight=0.5,
        symmetric=True,
        warn=True,
    ),
    "related": RelationSpec(reverse="related", weight=0.5, symmetric=True),
}


def weight(rel: str, config_override: dict | None = None) -> float:
    """Return the expansion weight for a relation."""
    overrides = config_override or {}
    if rel in overrides:
        return float(overrides[rel])
    spec = PREDEFINED.get(rel)
    return spec.weight if spec is not None else 0.5


def reverse(rel: str) -> str | None:
    spec = PREDEFINED.get(rel)
    return spec.reverse if spec is not None else None


def is_symmetric(rel: str) -> bool:
    spec = PREDEFINED.get(rel)
    return bool(spec and spec.symmetric)


def is_demoting(rel: str) -> bool:
    spec = PREDEFINED.get(rel)
    return bool(spec and spec.demote_target)


def warns(rel: str) -> bool:
    spec = PREDEFINED.get(rel)
    return bool(spec and spec.warn)
