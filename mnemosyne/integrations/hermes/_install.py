"""install-hermes helpers: copy provider files + edit config.yaml (no PyYAML).

config.yaml is machine-managed (no comments/anchors), so targeted line edits
preserve all unrelated content byte-for-byte.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from typing import List, Optional, Tuple

_PLUGIN_FILES = ("__init__.py", "plugin.yaml", "README.md")


def _top_block_bounds(lines: List[str], key: str) -> Tuple[Optional[int], int]:
    """Return (start, end) line indices of top-level ``key:`` block.

    start = index of the ``key:`` line; end = index of the next top-level
    line (or len(lines)). Returns (None, len(lines)) if absent.
    """
    start = None
    for i, ln in enumerate(lines):
        if ln[:1].isspace():
            continue
        head = ln.split(":", 1)[0].strip()
        if head == key and (ln.rstrip().endswith(":") or ln.startswith(key + ":")):
            start = i
            break
    if start is None:
        return None, len(lines)
    end = len(lines)
    for j in range(start + 1, len(lines)):
        ln = lines[j]
        if ln.strip() and not ln[:1].isspace():
            end = j
            break
    return start, end


def set_memory_provider(text: str, value: str = "mnemosyne") -> str:
    """Ensure ``memory.provider: <value>`` (replace / insert / create block)."""
    lines = text.splitlines(keepends=True)
    start, end = _top_block_bounds(lines, "memory")
    if start is None:
        suffix = "" if text.endswith("\n") or text == "" else "\n"
        return text + suffix + f"memory:\n  provider: {value}\n"
    for k in range(start + 1, end):
        stripped = lines[k].lstrip()
        if stripped.startswith("provider:"):
            indent = lines[k][: len(lines[k]) - len(stripped)]
            lines[k] = f"{indent}provider: {value}\n"
            return "".join(lines)
    lines.insert(start + 1, f"  provider: {value}\n")
    return "".join(lines)


_MNE_BLOCK = (
    "  mnemosyne:\n"
    "    python: {py}\n"
    "    recall_limit: 5\n"
    "    timeout: 5\n"
    "    mirror_builtin_writes: false\n"
)


def upsert_plugins_mnemosyne(text: str, python_path: str) -> str:
    """Ensure ``plugins.mnemosyne`` exists with the given python path."""
    block = _MNE_BLOCK.format(py=python_path)
    lines = text.splitlines(keepends=True)
    start, end = _top_block_bounds(lines, "plugins")
    if start is None:
        suffix = "" if text.endswith("\n") or text == "" else "\n"
        return text + suffix + "plugins:\n" + block
    sub_start = None
    for k in range(start + 1, end):
        if lines[k].rstrip("\n") == "  mnemosyne:" or lines[k].startswith("  mnemosyne:"):
            sub_start = k
            break
    if sub_start is None:
        new = lines[: start + 1] + [block] + lines[start + 1:]
        return "".join(new)
    sub_end = end
    for k in range(sub_start + 1, end):
        ln = lines[k]
        if ln.strip() and ln[:2] == "  " and ln[2:3] != " ":
            sub_end = k
            break
    new = lines[:sub_start] + [block] + lines[sub_end:]
    return "".join(new)


def detect_python(explicit: Optional[str] = None) -> Optional[str]:
    """Return an absolute path to a python that can ``import mnemosyne``."""
    candidates = [explicit, shutil.which("python3"), "/opt/homebrew/bin/python3", sys.executable]
    for cand in candidates:
        if not cand:
            continue
        try:
            proc = subprocess.run([cand, "-c", "import mnemosyne"],
                                  capture_output=True, text=True, timeout=10)
            if proc.returncode == 0:
                return os.path.abspath(cand)
        except Exception:
            continue
    return None


def _provider_src_dir() -> Path:
    return Path(__file__).resolve().parent


def install_hermes(*, hermes_home: Path, python_path: Optional[str] = None,
                   force: bool = False, write_config: bool = True,
                   dry_run: bool = False) -> dict:
    """Copy provider files into HERMES_HOME and (optionally) edit config.yaml."""
    hermes_home = Path(hermes_home)
    src = _provider_src_dir()
    plugin_dir = hermes_home / "plugins" / "mnemosyne"
    py = detect_python(python_path) or python_path or sys.executable

    plan = {"plugin_dir": str(plugin_dir), "python": py,
            "config_written": False, "backup": None, "dry_run": dry_run}

    if plugin_dir.exists() and not force and not dry_run:
        raise FileExistsError(
            f"{plugin_dir} already exists — re-run with --force to overwrite")

    if not dry_run:
        plugin_dir.mkdir(parents=True, exist_ok=True)
        for fname in _PLUGIN_FILES:
            shutil.copy2(src / fname, plugin_dir / fname)

    if write_config:
        cfg_path = hermes_home / "config.yaml"
        original = cfg_path.read_text(encoding="utf-8") if cfg_path.exists() else ""
        updated = upsert_plugins_mnemosyne(set_memory_provider(original), py)
        if not dry_run:
            if cfg_path.exists():
                ts = datetime.now().strftime("%Y%m%d%H%M%S")
                backup = cfg_path.with_name(f"config.yaml.mnemosyne-bak-{ts}")
                shutil.copy2(cfg_path, backup)
                plan["backup"] = str(backup)
            cfg_path.write_text(updated, encoding="utf-8")
            plan["config_written"] = True
        else:
            plan["config_preview"] = updated

    return plan
