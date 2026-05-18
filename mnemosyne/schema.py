"""Memory schema and frontmatter parsing."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


FRONTMATTER_ORDER = [
    "id",
    "type",
    "source",
    "strength",
    "created",
    "last_accessed",
    "access_count",
    "tags",
    "links",
    "canonical_summary",
    "injection_summary",
    "status",
    "expires",
]


@dataclass
class Memory:
    id: str
    type: str
    source: str = "agent"
    strength: int = 0
    created: str = ""
    last_accessed: str = ""
    access_count: int = 0
    tags: list[str] = field(default_factory=list)
    links: list[dict[str, str]] = field(default_factory=list)
    canonical_summary: str = ""
    injection_summary: str = ""
    status: str = "active"
    body: str = ""
    expires: str = ""
    extra: dict[str, Any] = field(default_factory=dict)

    @property
    def title(self) -> str:
        for line in self.body.splitlines():
            stripped = line.strip()
            if stripped.startswith("#"):
                return stripped.lstrip("#").strip()
        return self.injection_summary or self.canonical_summary or self.id

    def to_frontmatter(self) -> dict[str, Any]:
        data: dict[str, Any] = {
            "id": self.id,
            "type": self.type,
            "source": self.source,
            "strength": self.strength,
            "created": self.created,
            "last_accessed": self.last_accessed,
            "access_count": self.access_count,
            "tags": self.tags,
            "links": self.links,
            "canonical_summary": self.canonical_summary,
            "injection_summary": self.injection_summary,
            "status": self.status,
        }
        if self.expires:
            data["expires"] = self.expires
        for key, value in self.extra.items():
            if key not in data:
                data[key] = value
        return data


def parse_memory(text: str) -> Memory:
    frontmatter, body = split_frontmatter(text)
    data = parse_frontmatter(frontmatter)
    known = {key: data.pop(key) for key in list(data) if key in FRONTMATTER_ORDER}
    return Memory(
        id=str(known.get("id", "")),
        type=str(known.get("type", "codebase")),
        source=str(known.get("source", "agent")),
        strength=int(known.get("strength", 0) or 0),
        created=str(known.get("created", "")),
        last_accessed=str(known.get("last_accessed", "")),
        access_count=int(known.get("access_count", 0) or 0),
        tags=[str(item) for item in known.get("tags", []) or []],
        links=_normalize_links(known.get("links", []) or []),
        canonical_summary=str(known.get("canonical_summary", "")),
        injection_summary=str(known.get("injection_summary", "")),
        status=str(known.get("status", "active")),
        body=body.strip(),
        expires=str(known.get("expires", "")),
        extra=data,
    )


def serialize_memory(memory: Memory) -> str:
    return serialize_frontmatter(memory.to_frontmatter()) + "\n" + memory.body.strip() + "\n"


def split_frontmatter(text: str) -> tuple[str, str]:
    normalized = text.replace("\r\n", "\n")
    if not normalized.startswith("---\n"):
        return "", normalized
    end = normalized.find("\n---\n", 4)
    if end == -1:
        return "", normalized
    frontmatter = normalized[4:end]
    body = normalized[end + 5 :]
    return frontmatter, body


def parse_frontmatter(text: str) -> dict[str, Any]:
    data: dict[str, Any] = {}
    lines = text.replace("\r\n", "\n").splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        if not line.strip() or line.startswith(" "):
            i += 1
            continue
        if ":" not in line:
            i += 1
            continue

        key, raw_value = line.split(":", 1)
        key = key.strip()
        raw_value = raw_value.strip()

        if raw_value:
            data[key] = parse_value(raw_value)
            i += 1
            continue

        block: list[Any] = []
        i += 1
        current_dict: dict[str, Any] | None = None
        while i < len(lines):
            child = lines[i]
            if not child.startswith(" ") and child.strip():
                break
            stripped = child.strip()
            if not stripped:
                i += 1
                continue

            if stripped.startswith("- "):
                item = stripped[2:].strip()
                if ":" in item:
                    item_key, item_value = item.split(":", 1)
                    current_dict = {item_key.strip(): parse_value(item_value.strip())}
                    block.append(current_dict)
                else:
                    current_dict = None
                    block.append(parse_value(item))
                i += 1
                continue

            if current_dict is not None and ":" in stripped:
                item_key, item_value = stripped.split(":", 1)
                current_dict[item_key.strip()] = parse_value(item_value.strip())
            i += 1

        data[key] = block

    return data


def parse_value(value: str) -> Any:
    value = value.strip()
    if value == "":
        return ""
    if value.startswith("[") and value.endswith("]"):
        inner = value[1:-1].strip()
        if not inner:
            return []
        return [parse_value(part.strip()) for part in inner.split(",")]
    if (value.startswith("'") and value.endswith("'")) or (
        value.startswith('"') and value.endswith('"')
    ):
        return value[1:-1]
    lowered = value.lower()
    if lowered == "true":
        return True
    if lowered == "false":
        return False
    if lowered == "null":
        return None
    try:
        return int(value)
    except ValueError:
        return value


def serialize_frontmatter(data: dict[str, Any]) -> str:
    lines = ["---"]
    seen: set[str] = set()
    for key in FRONTMATTER_ORDER:
        if key in data:
            _append_value(lines, key, data[key])
            seen.add(key)
    for key in sorted(data):
        if key not in seen:
            _append_value(lines, key, data[key])
    lines.append("---")
    return "\n".join(lines)


def _append_value(lines: list[str], key: str, value: Any) -> None:
    if isinstance(value, list):
        if not value:
            lines.append(f"{key}: []")
            return
        if all(not isinstance(item, dict) for item in value):
            inner = ", ".join(_format_scalar(item) for item in value)
            lines.append(f"{key}: [{inner}]")
            return
        lines.append(f"{key}:")
        for item in value:
            if isinstance(item, dict):
                item_lines = list(item.items())
                if not item_lines:
                    lines.append("  - {}")
                    continue
                first_key, first_value = item_lines[0]
                lines.append(f"  - {first_key}: {_format_scalar(first_value)}")
                for child_key, child_value in item_lines[1:]:
                    lines.append(f"    {child_key}: {_format_scalar(child_value)}")
            else:
                lines.append(f"  - {_format_scalar(item)}")
        return
    lines.append(f"{key}: {_format_scalar(value)}")


def _format_scalar(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    text = str(value).replace("\n", " ").strip()
    if text == "":
        return ""
    if any(ch in text for ch in [":", "#", "[", "]", "{", "}", ","]) or text != text.strip():
        escaped = text.replace('"', '\\"')
        return f'"{escaped}"'
    return text


def _normalize_links(value: Any) -> list[dict[str, str]]:
    links: list[dict[str, str]] = []
    if not isinstance(value, list):
        return links
    for item in value:
        if isinstance(item, dict):
            link_id = str(item.get("id", "")).strip()
            rel = str(item.get("rel", "")).strip()
            if link_id:
                links.append({"id": link_id, "rel": rel})
    return links
