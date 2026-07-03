"""Path resolution, configuration, and memory file I/O."""

from __future__ import annotations

import os
import shutil
from copy import deepcopy
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import portalocker

from mnemosyne.schema import Memory, parse_memory, serialize_memory


DEFAULT_CONFIG = {
    "thresholds": {
        "decay_per_run": 1,
        "bonus_access": 5,
        "bonus_write": 10,
        "bonus_recall": 20,
        "core_strength": 80,
        "core_access_count": 3,
        "archive_strength": 30,
        "deprecated_strength": 5,
    },
    "memory": {
        "types": ["arch_decision", "pitfall", "codebase", "preference", "handoff", "session_summary"],
    },
    "injection": {
        "max_tokens": 2000,
        "summary_chars": 120,
    },
    "search": {
        "index_enabled": True,
    },
    "embedding": {
        "enabled": False,
        "backend": "onnx",
        "model": "BAAI/bge-small-zh-v1.5",
        "onnx_path": "",
        "api_base": "https://api.openai.com/v1",
        "api_key_env": "OPENAI_API_KEY",
        "dimensions": 384,
        "batch_size": 32,
    },
    "rerank": {
        "enabled": False,
        "backend": "cross_encoder",
        "model": "BAAI/bge-reranker-base",
        "onnx_path": "",
        "top_n": 5,
    },
    "distill": {
        "enabled": False,
        "engine": "heuristic",
        "session_summary": False,
        "confidence_threshold": 0.6,
        "max_findings_per_session": 5,
        "dedup_threshold": 0.85,
        "subject_threshold": 0.5,
        "llm": {
            "backend": "openai",
            "model": "",
            "api_base": "https://api.openai.com/v1",
            "api_key_env": "OPENAI_API_KEY",
        },
    },
    "fusion": {
        "rrf_k": 60,
        "link_expansion": True,
        "link_expansion_decay_fallback": 0.5,
        "link_expansion_max_hops": 1,
        "bm25_pool_size": 50,
        "vec_pool_size": 50,
    },
    "relations": {
        "allow_custom": False,
    },
    "mcp": {
        "expose_global": True,
        "expose_project": True,
        "default_search_limit": 5,
        "sse": {
            "enabled": False,
            "port": 3700,
            "host": "127.0.0.1",
        },
    },
}


DEFAULT_CONFIG_TOML = """[thresholds]
decay_per_run = 1
bonus_access = 5
bonus_write = 10
bonus_recall = 20
core_strength = 80
core_access_count = 3
archive_strength = 30
deprecated_strength = 5

[memory]
types = ['arch_decision', 'pitfall', 'codebase', 'preference', 'handoff', 'session_summary']

[injection]
max_tokens = 2000
summary_chars = 120

[search]
index_enabled = true

[embedding]
enabled = false
backend = "onnx"
model = "BAAI/bge-small-zh-v1.5"
onnx_path = ""
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
dimensions = 384
batch_size = 32

[rerank]
enabled = false
backend = "cross_encoder"
model = "BAAI/bge-reranker-base"
onnx_path = ""
top_n = 5

[distill]
enabled = false
engine = "heuristic"
session_summary = false
confidence_threshold = 0.6
max_findings_per_session = 5
dedup_threshold = 0.85
subject_threshold = 0.5

[distill.llm]
backend = "openai"
model = ""
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[fusion]
rrf_k = 60
link_expansion = true
link_expansion_decay_fallback = 0.5
link_expansion_max_hops = 1
bm25_pool_size = 50
vec_pool_size = 50

[relations]
allow_custom = false

[mcp]
expose_global = true
expose_project = true
default_search_limit = 5

[mcp.sse]
enabled = false
port = 3700
host = "127.0.0.1"
"""


@dataclass(frozen=True)
class Store:
    scope: str
    root: Path

    @property
    def core_path(self) -> Path:
        return self.root / "core.md"

    @property
    def working_dir(self) -> Path:
        return self.root / "working"

    @property
    def archive_dir(self) -> Path:
        return self.root / "archive"

    @property
    def config_path(self) -> Path:
        return self.root / "config.toml"


def global_store() -> Store:
    root = Path(os.environ.get("MNEMOSYNE_HOME", "~/.mnemosyne")).expanduser()
    return Store("global", root)


def find_project_store(start: Path | None = None) -> Store | None:
    current = (start or Path.cwd()).resolve()
    if current.is_file():
        current = current.parent
    global_root = global_store().root.resolve()
    while True:
        candidate = current / ".mnemosyne"
        # Never treat the global store dir as a project store: when cwd is $HOME
        # (and ~/.mnemosyne is the global store), this used to mislabel global as
        # a project, so project-scoped writes silently mutated the global store.
        if candidate.exists() and candidate.resolve() != global_root:
            return Store("project", candidate)
        if (current / ".git").exists():
            return None
        parent = current.parent
        if parent == current:
            return None
        current = parent


def project_store(start: Path | None = None) -> Store:
    found = find_project_store(start)
    if found is not None:
        return found
    root = (start or Path.cwd()).resolve() / ".mnemosyne"
    return Store("project", root)


def stores_for_scope(scope: str) -> list[Store]:
    if scope == "global":
        return [global_store()]
    if scope == "project":
        return [project_store()]
    if scope == "all":
        stores = [global_store()]
        found = find_project_store()
        if found is not None:
            stores.append(found)
        return stores
    raise ValueError(f"unknown scope: {scope}")


def ensure_store(store: Store, core_template: str | None = None) -> None:
    store.working_dir.mkdir(parents=True, exist_ok=True)
    store.archive_dir.mkdir(parents=True, exist_ok=True)
    if not store.core_path.exists():
        store.core_path.write_text(core_template or "", encoding="utf-8")
    if store.scope == "project" and not store.config_path.exists():
        store.config_path.write_text(DEFAULT_CONFIG_TOML, encoding="utf-8")


def load_config(store: Store | None = None) -> dict:
    config = _deepcopy_default_config()
    if store is None:
        found = find_project_store()
        store = found if found is not None else global_store()
    if store.config_path.exists():
        try:
            import tomllib
        except ModuleNotFoundError:
            return config
        with store.config_path.open("rb") as handle:
            loaded = tomllib.load(handle)
        _merge_config(config, loaded)
    return config


def read_core(store: Store) -> str:
    if not store.core_path.exists():
        return ""
    return store.core_path.read_text(encoding="utf-8")


def iter_memory_paths(store: Store, include_archive: bool = False) -> Iterable[Path]:
    if store.working_dir.exists():
        yield from sorted(store.working_dir.glob("*.md"))
    if include_archive and store.archive_dir.exists():
        yield from sorted(store.archive_dir.glob("*/*.md"))


def load_memories(store: Store, include_archive: bool = False) -> list[tuple[Path, Memory]]:
    memories: list[tuple[Path, Memory]] = []
    for path in iter_memory_paths(store, include_archive=include_archive):
        try:
            memories.append((path, parse_memory(path.read_text(encoding="utf-8"))))
        except (OSError, ValueError):
            # ValueError covers UnicodeDecodeError: memories are hand-editable
            # files, and one corrupt file must not take down the whole store.
            continue
    return memories


def corrupt_memory_paths(store: Store, include_archive: bool = True) -> list[Path]:
    """Memory files that cannot be used: unreadable, undecodable, or missing an id."""
    bad: list[Path] = []
    for path in iter_memory_paths(store, include_archive=include_archive):
        try:
            memory = parse_memory(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            bad.append(path)
            continue
        if not memory.id:
            bad.append(path)
    return bad


def write_memory(path: Path, memory: Memory, lock_timeout: float = 10.0) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lock_path = path.with_suffix(path.suffix + ".lock")
    tmp_path = path.with_suffix(path.suffix + ".tmp")
    with portalocker.Lock(str(lock_path), mode="a", timeout=lock_timeout):
        tmp_path.write_text(serialize_memory(memory), encoding="utf-8")
        os.replace(tmp_path, path)
    try:
        lock_path.unlink()
    except OSError:
        pass


@contextmanager
def lock_store(store: Store, timeout: float = 30.0):
    store.root.mkdir(parents=True, exist_ok=True)
    lock_path = store.root / ".lock"
    with portalocker.Lock(str(lock_path), mode="a", timeout=timeout):
        yield


def memory_filename(memory: Memory) -> str:
    safe_id = "".join(ch if ch.isalnum() or ch in "_-" else "_" for ch in memory.id)
    return f"{safe_id}.md"


def working_path(store: Store, memory: Memory) -> Path:
    return store.working_dir / memory_filename(memory)


def archive_path(store: Store, memory: Memory, yyyy_mm: str) -> Path:
    return store.archive_dir / yyyy_mm / memory_filename(memory)


def find_memory(memory_id: str, stores: Iterable[Store], include_archive: bool = True) -> tuple[Store, Path, Memory] | None:
    for store in stores:
        for path, memory in load_memories(store, include_archive=include_archive):
            if memory.id == memory_id:
                return store, path, memory
    return None


def move_to_archive(store: Store, path: Path, memory: Memory, yyyy_mm: str) -> Path:
    destination = archive_path(store, memory, yyyy_mm)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(path), str(destination))
    return destination


def template_text(name: str) -> str:
    path = Path(__file__).resolve().parent.parent / "templates" / name
    return path.read_text(encoding="utf-8")


def _deepcopy_default_config() -> dict:
    return deepcopy(DEFAULT_CONFIG)


def _merge_config(config: dict, loaded: dict) -> None:
    for section, values in loaded.items():
        if not isinstance(values, dict):
            continue
        target = config.setdefault(section, {})
        for key, value in values.items():
            target[key] = value
