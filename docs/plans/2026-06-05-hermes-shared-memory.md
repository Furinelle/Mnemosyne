# Hermes 共享 Mnemosyne 记忆 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Hermes 通过一个原生 `MemoryProvider` 插件读写同一个 Mnemosyne 全局 store，实现与 Claude Code、Codex 三端记忆共享。

**Architecture:** 在本仓库新增 `mnemosyne/integrations/hermes/` —— 一个单文件 `MemoryProvider`，通过子进程调 `python3 -m mnemosyne <cmd>` 操作共享 store（读 core 注入系统提示、每轮检索 prefetch、暴露单个 `mnemosyne` 工具供写入）。新增 CLI 子命令 `install-hermes` 把插件拷到 `~/.hermes/plugins/mnemosyne/` 并定向改写 `config.yaml` 激活。

**Tech Stack:** Python ≥3.11，stdlib only（subprocess/json/os/shutil/re），无 PyYAML、无新依赖。参考设计文档 `docs/specs/2026-06-05-hermes-shared-memory-design.md`。

---

## File Structure

- `mnemosyne/integrations/__init__.py` — 包标记（空 docstring）。
- `mnemosyne/integrations/hermes/__init__.py` — provider 模块（被 `install-hermes` 拷到插件目录）。唯一职责：把 Hermes `MemoryProvider` 生命周期映射到 Mnemosyne CLI。单文件、可独立 import（无 Hermes 时用 shim）。
- `mnemosyne/integrations/hermes/plugin.yaml` — Hermes 插件清单。
- `mnemosyne/integrations/hermes/README.md` — 安装/配置说明。
- `mnemosyne/integrations/hermes/_install.py` — 安装逻辑 + config 文本编辑纯函数（**不**拷到插件目录，仅供 CLI 调用与测试）。
- `mnemosyne/cli.py` — 新增 `install-hermes` 子命令解析与分派。
- `tests/test_hermes_provider.py` — provider 行为测试。
- `tests/test_hermes_install.py` — config 编辑纯函数 + 安装命令测试。
- `README.md` — 补充 Hermes 接入章节。

---

## Task 1: 包骨架 + provider 可独立 import

**Files:**
- Create: `mnemosyne/integrations/__init__.py`
- Create: `mnemosyne/integrations/hermes/__init__.py`
- Test: `tests/test_hermes_provider.py`

- [ ] **Step 1: 写失败测试**

```python
# tests/test_hermes_provider.py
import sys
from pathlib import Path

from mnemosyne import store as mstore
from mnemosyne.integrations.hermes import MnemosyneMemoryProvider, register


def test_name_is_mnemosyne():
    p = MnemosyneMemoryProvider(config={})
    assert p.name == "mnemosyne"


def test_register_collects_provider():
    class Ctx:
        def __init__(self):
            self.provider = None
        def register_memory_provider(self, provider):
            self.provider = provider
    ctx = Ctx()
    register(ctx)
    assert ctx.provider is not None
    assert ctx.provider.name == "mnemosyne"
```

- [ ] **Step 2: 运行测试确认失败**

Run: `python -m pytest tests/test_hermes_provider.py -q`
Expected: FAIL（`ModuleNotFoundError: No module named 'mnemosyne.integrations'`）

- [ ] **Step 3: 创建包标记**

```python
# mnemosyne/integrations/__init__.py
"""Adapters that bridge Mnemosyne to external agents (Hermes, etc.)."""
```

- [ ] **Step 4: 写 provider 骨架（可独立 import）**

```python
# mnemosyne/integrations/hermes/__init__.py
"""Hermes MemoryProvider that bridges to the shared Mnemosyne store via CLI.

Installed into ``$HERMES_HOME/plugins/mnemosyne/`` by
``python3 -m mnemosyne install-hermes``. Activated via ``memory.provider:
mnemosyne`` in Hermes ``config.yaml``.

Importable standalone (for tests) — the Hermes-only base class and helpers
fall back to local shims when Hermes is not on the path.
"""
from __future__ import annotations

import json
import logging
import os
import shutil
import subprocess
import sys
from typing import Any, Dict, List, Optional

logger = logging.getLogger(__name__)

try:  # Hermes runtime
    from agent.memory_provider import MemoryProvider  # type: ignore
except Exception:  # standalone / tests
    class MemoryProvider:  # minimal shim
        pass

try:  # Hermes runtime
    from tools.registry import tool_error  # type: ignore
except Exception:  # standalone / tests
    def tool_error(msg: str) -> str:
        return json.dumps({"error": msg})


class MnemosyneMemoryProvider(MemoryProvider):
    """Bridges Hermes memory lifecycle to the shared Mnemosyne CLI."""

    def __init__(self, config: Optional[Dict[str, Any]] = None):
        self._config = config or {}
        self._python: Optional[str] = None
        self._python_resolved = False
        self._session_id = ""
        self._source = "hermes"
        self._recall_limit = 5
        self._timeout = 5.0
        self._mirror = False

    @property
    def name(self) -> str:
        return "mnemosyne"

    def is_available(self) -> bool:  # filled in Task 2
        return False

    def initialize(self, session_id: str, **kwargs) -> None:  # filled in Task 2
        self._session_id = session_id or ""

    def get_tool_schemas(self) -> List[Dict[str, Any]]:  # filled in Task 4
        return []


def _load_plugin_config() -> Dict[str, Any]:
    """Read ``plugins.mnemosyne`` from Hermes config.yaml (best-effort)."""
    try:
        import yaml  # Hermes venv has PyYAML
        from hermes_constants import get_hermes_home  # type: ignore
        cfg_path = get_hermes_home() / "config.yaml"
        if not cfg_path.exists():
            return {}
        with open(cfg_path, encoding="utf-8-sig") as fh:
            data = yaml.safe_load(fh) or {}
        return (data.get("plugins") or {}).get("mnemosyne") or {}
    except Exception:
        return {}


def register(ctx) -> None:
    """Hermes plugin entry point."""
    ctx.register_memory_provider(MnemosyneMemoryProvider(config=_load_plugin_config()))
```

- [ ] **Step 5: 运行测试确认通过**

Run: `python -m pytest tests/test_hermes_provider.py -q`
Expected: PASS（2 passed）

- [ ] **Step 6: 提交**

```bash
git add mnemosyne/integrations/__init__.py mnemosyne/integrations/hermes/__init__.py tests/test_hermes_provider.py
git commit -m "feat(hermes): scaffold Mnemosyne memory provider"
```

---

## Task 2: 子进程桥接 `_run` + python 探测 + `is_available` + `initialize`

**Files:**
- Modify: `mnemosyne/integrations/hermes/__init__.py`
- Test: `tests/test_hermes_provider.py`

- [ ] **Step 1: 写失败测试**

测试顶部加共享 helper（放文件靠上、import 之后）：

```python
import os
import subprocess
import pytest


def _seed_global(tmp_path, monkeypatch, core_text="- shared core line"):
    """Redirect the global store to tmp and seed core.md."""
    home = tmp_path / ".mnemosyne"
    monkeypatch.setenv("MNEMOSYNE_HOME", str(home))
    gs = mstore.global_store()
    mstore.ensure_store(gs)
    gs.core_path.write_text("# Global Core Memory\n\n" + core_text + "\n", encoding="utf-8")
    return gs


def _provider(**cfg):
    base = {"python": sys.executable, "timeout": 30}
    base.update(cfg)
    return MnemosyneMemoryProvider(config=base)
```

新增用例：

```python
def test_is_available_true_with_real_python(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1")
    assert p.is_available() is True


def test_is_available_false_with_bad_python(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider(python="/nonexistent/python-xyz")
    assert p.is_available() is False


def test_run_returns_empty_on_failure(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider(python="/nonexistent/python-xyz")
    assert p._run(["read", "--scope", "all"]) == ""
    assert p._run(["search", "x", "--format", "json"], json_out=True) == []


def test_initialize_sets_source_from_profile(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1", agent_identity="coder")
    assert p._source == "hermes:coder"
```

- [ ] **Step 2: 运行确认失败**

Run: `python -m pytest tests/test_hermes_provider.py -q`
Expected: FAIL（`is_available` 返回 False / `_run` 不存在 → AttributeError）

- [ ] **Step 3: 实现桥接与生命周期**

在 `MnemosyneMemoryProvider` 内，用下列方法替换 Task 1 的 `is_available`/`initialize` 占位，并新增 `_resolve_python`/`_python_has_mnemosyne`/`_run`：

```python
    # -- bridge ---------------------------------------------------------------

    def _python_has_mnemosyne(self, py: str) -> bool:
        if not py:
            return False
        try:
            proc = subprocess.run(
                [py, "-c", "import mnemosyne"],
                capture_output=True, text=True, timeout=10,
            )
            return proc.returncode == 0
        except Exception:
            return False

    def _resolve_python(self) -> Optional[str]:
        if self._python_resolved:
            return self._python
        self._python_resolved = True
        candidates = [
            self._config.get("python"),
            shutil.which("python3"),
            "/opt/homebrew/bin/python3",
            sys.executable,
        ]
        for cand in candidates:
            if cand and self._python_has_mnemosyne(cand):
                self._python = cand
                return cand
        self._python = None
        return None

    def _run(self, args: List[str], *, json_out: bool = False):
        empty = [] if json_out else ""
        py = self._resolve_python()
        if not py:
            return empty
        try:
            proc = subprocess.run(
                [py, "-m", "mnemosyne", *args],
                capture_output=True, text=True,
                timeout=self._timeout, cwd=os.getcwd(),
            )
        except Exception as exc:
            logger.debug("mnemosyne %s failed: %s", args[:1], exc)
            return empty
        if proc.returncode != 0:
            logger.debug("mnemosyne %s exit %s: %s", args[:1], proc.returncode, proc.stderr[:200])
            return empty
        out = proc.stdout
        if json_out:
            try:
                return json.loads(out) if out.strip() else []
            except Exception:
                return []
        return out

    # -- lifecycle ------------------------------------------------------------

    def is_available(self) -> bool:
        return self._resolve_python() is not None

    def initialize(self, session_id: str, **kwargs) -> None:
        self._session_id = session_id or ""
        self._recall_limit = int(self._config.get("recall_limit", 5))
        self._timeout = float(self._config.get("timeout", 5))
        self._mirror = bool(self._config.get("mirror_builtin_writes", False))
        profile = kwargs.get("agent_identity")
        self._source = f"hermes:{profile}" if profile else "hermes"
        self._resolve_python()

    def shutdown(self) -> None:
        pass
```

> 注：`_run` 默认 timeout 在 `initialize` 前为 5.0；`is_available` 早于 `initialize` 被调时仅用到 `_python_has_mnemosyne`（10s 独立 timeout），不受影响。

- [ ] **Step 4: 运行确认通过**

Run: `python -m pytest tests/test_hermes_provider.py -q`
Expected: PASS（6 passed）

- [ ] **Step 5: 提交**

```bash
git add mnemosyne/integrations/hermes/__init__.py tests/test_hermes_provider.py
git commit -m "feat(hermes): add CLI bridge, python detection, lifecycle"
```

---

## Task 3: `system_prompt_block` + `prefetch`（真实子进程读 store）

**Files:**
- Modify: `mnemosyne/integrations/hermes/__init__.py`
- Test: `tests/test_hermes_provider.py`

- [ ] **Step 1: 写失败测试**

文件靠上新增 CLI helper：

```python
def _cli(*args):
    subprocess.run([sys.executable, "-m", "mnemosyne", *args],
                   check=True, capture_output=True, text=True)
```

用例：

```python
def test_system_prompt_block_includes_core(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch, core_text="- remember the alamo")
    p = _provider()
    p.initialize("sess-1")
    block = p.system_prompt_block()
    assert "Mnemosyne Shared Memory" in block
    assert "remember the alamo" in block


def test_system_prompt_block_empty_when_no_store(tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "empty-home"))
    p = _provider()
    p.initialize("sess-1")
    assert p.system_prompt_block() == ""


def test_prefetch_returns_matching_memory(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    _cli("write", "--scope", "global", "--type", "codebase", "--importance", "70",
         "--source", "test", "--force", "--title", "Widget service entrypoint",
         "--content", "The widget service starts in widget_main.py and binds port 9090.",
         "--tags", "widget,service")
    p = _provider()
    p.initialize("sess-1")
    out = p.prefetch("widget service port")
    assert "Mnemosyne recall" in out
    assert "widget" in out.lower()


def test_prefetch_empty_query_returns_empty(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1")
    assert p.prefetch("   ") == ""
```

- [ ] **Step 2: 运行确认失败**

Run: `python -m pytest tests/test_hermes_provider.py -k "system_prompt or prefetch" -q`
Expected: FAIL（方法返回默认值/不存在）

- [ ] **Step 3: 实现两方法**

在类中新增：

```python
    def system_prompt_block(self) -> str:
        text = (self._run(["read", "--scope", "all"]) or "").strip()
        if not text:
            return ""
        return "# Mnemosyne Shared Memory\n\n" + text

    def prefetch(self, query: str, *, session_id: str = "") -> str:
        query = (query or "").strip()
        if not query:
            return ""
        rows = self._run(
            ["search", query, "--format", "json", "--limit", str(self._recall_limit)],
            json_out=True,
        )
        if not rows:
            return ""
        lines = ["## Mnemosyne recall"]
        for row in rows:
            summary = (row.get("summary") or "").strip()
            if not summary:
                continue
            try:
                score = float(row.get("score", 0))
            except (TypeError, ValueError):
                score = 0.0
            mtype = row.get("type", "")
            lines.append(f"- [{score:.1f}] ({mtype}) {summary}")
        return "\n".join(lines) if len(lines) > 1 else ""

    def sync_turn(self, user_content: str, assistant_content: str, *,
                  session_id: str = "", messages=None) -> None:
        # writes go through the explicit `mnemosyne` tool, not auto-sync
        pass
```

- [ ] **Step 4: 运行确认通过**

Run: `python -m pytest tests/test_hermes_provider.py -q`
Expected: PASS（10 passed）

- [ ] **Step 5: 提交**

```bash
git add mnemosyne/integrations/hermes/__init__.py tests/test_hermes_provider.py
git commit -m "feat(hermes): inject core via system_prompt_block + prefetch recall"
```

---

## Task 4: 工具面 `get_tool_schemas` + `handle_tool_call` + `on_memory_write`

**Files:**
- Modify: `mnemosyne/integrations/hermes/__init__.py`
- Test: `tests/test_hermes_provider.py`

- [ ] **Step 1: 写失败测试**

```python
def test_tool_schema_is_single_mnemosyne_tool(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1")
    schemas = p.get_tool_schemas()
    assert len(schemas) == 1
    assert schemas[0]["name"] == "mnemosyne"
    assert "action" in schemas[0]["parameters"]["properties"]


def test_write_then_search_roundtrip(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1", agent_identity="coder")
    w = json.loads(p.handle_tool_call("mnemosyne", {
        "action": "write", "type": "preference", "importance": 65,
        "scope": "global", "title": "Indentation preference",
        "content": "User prefers tabs over spaces in Go code.",
        "tags": "style,go",
    }))
    assert w["status"] == "ok"
    s = json.loads(p.handle_tool_call("mnemosyne", {"action": "search", "query": "tabs spaces go"}))
    assert s["count"] >= 1


def test_write_requires_type_and_importance(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1")
    r = json.loads(p.handle_tool_call("mnemosyne", {"action": "write", "title": "x"}))
    assert "error" in r


def test_unknown_action_errors(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider()
    p.initialize("sess-1")
    r = json.loads(p.handle_tool_call("mnemosyne", {"action": "frobnicate"}))
    assert "error" in r


def test_on_memory_write_mirrors_when_enabled(tmp_path, monkeypatch):
    _seed_global(tmp_path, monkeypatch)
    p = _provider(mirror_builtin_writes=True)
    p.initialize("sess-1")
    p.on_memory_write("add", "user", "User lives in the UTC+8 timezone.")
    s = json.loads(p.handle_tool_call("mnemosyne", {"action": "search", "query": "timezone UTC+8"}))
    assert s["count"] >= 1
```

- [ ] **Step 2: 运行确认失败**

Run: `python -m pytest tests/test_hermes_provider.py -k "tool or write or action or mirror" -q`
Expected: FAIL（`get_tool_schemas` 返回 []，`handle_tool_call` 抛 NotImplementedError）

- [ ] **Step 3: 实现工具 schema 与分派**

模块级（类外，logger 之后）加 schema 常量：

```python
MNEMOSYNE_TOOL: Dict[str, Any] = {
    "name": "mnemosyne",
    "description": (
        "Shared long-term memory across Claude Code, Codex, and Hermes. "
        "action='search' to recall past memories; action='write' to persist a "
        "durable fact the user would expect remembered across sessions and agents "
        "(preferences, decisions, pitfalls, codebase knowledge). "
        "Also: show / link / graph."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["search", "write", "show", "link", "graph"]},
            "query": {"type": "string", "description": "search query (action=search)"},
            "limit": {"type": "integer", "description": "max results (action=search)"},
            "type": {"type": "string", "description": "preference|arch_decision|pitfall|codebase|handoff (action=write)"},
            "importance": {"type": "integer", "description": "50-90 (action=write)"},
            "scope": {"type": "string", "enum": ["global", "project"], "description": "write scope (default global)"},
            "title": {"type": "string"},
            "content": {"type": "string"},
            "tags": {"type": "string", "description": "comma-separated"},
            "id": {"type": "string", "description": "memory id (action=show|graph|link)"},
            "id2": {"type": "string", "description": "second id (action=link)"},
            "rel": {"type": "string", "description": "caused_by|refines|supersedes|contradicts|related (action=link)"},
        },
        "required": ["action"],
    },
}
```

类中替换 Task 1 的 `get_tool_schemas` 占位，并新增分派：

```python
    def get_tool_schemas(self) -> List[Dict[str, Any]]:
        return [MNEMOSYNE_TOOL]

    def handle_tool_call(self, tool_name: str, args: Dict[str, Any], **kwargs) -> str:
        if tool_name != "mnemosyne":
            return tool_error(f"Unknown tool: {tool_name}")
        action = args.get("action")
        if action == "search":
            query = (args.get("query") or "").strip()
            if not query:
                return tool_error("search requires 'query'")
            limit = int(args.get("limit", self._recall_limit))
            rows = self._run(["search", query, "--format", "json", "--limit", str(limit)], json_out=True)
            return json.dumps({"results": rows, "count": len(rows)})
        if action == "write":
            if not args.get("type") or args.get("importance") in (None, ""):
                return tool_error("write requires 'type' and 'importance'")
            cmd = ["write", "--force", "--source", self._source,
                   "--type", str(args["type"]),
                   "--importance", str(args["importance"]),
                   "--scope", str(args.get("scope", "global"))]
            for flag, key in (("--title", "title"), ("--content", "content"), ("--tags", "tags")):
                val = args.get(key)
                if val is not None and str(val) != "":
                    cmd += [flag, str(val)]
            out = self._run(cmd)
            if not out:
                return tool_error("write failed")
            return json.dumps({"status": "ok", "output": out.strip()})
        if action == "show":
            mid = args.get("id")
            if not mid:
                return tool_error("show requires 'id'")
            return json.dumps({"memory": self._run(["show", str(mid)])})
        if action == "link":
            i, j, rel = args.get("id"), args.get("id2"), args.get("rel")
            if not (i and j and rel):
                return tool_error("link requires 'id', 'id2', 'rel'")
            out = self._run(["link", str(i), str(j), "--rel", str(rel)])
            return json.dumps({"status": "ok", "output": out.strip()})
        if action == "graph":
            mid = args.get("id")
            if not mid:
                return tool_error("graph requires 'id'")
            return json.dumps({"graph": self._run(["graph", str(mid), "--format", "mermaid"])})
        return tool_error(f"Unknown action: {action}")

    def on_memory_write(self, action: str, target: str, content: str,
                        metadata: Optional[Dict[str, Any]] = None) -> None:
        if not self._mirror or action != "add" or not (content or "").strip():
            return
        mtype = "preference" if target == "user" else "codebase"
        self._run(["write", "--force", "--source", self._source, "--scope", "global",
                   "--type", mtype, "--importance", "60",
                   "--title", content[:60], "--content", content])
```

- [ ] **Step 4: 运行确认通过**

Run: `python -m pytest tests/test_hermes_provider.py -q`
Expected: PASS（15 passed）

- [ ] **Step 5: 提交**

```bash
git add mnemosyne/integrations/hermes/__init__.py tests/test_hermes_provider.py
git commit -m "feat(hermes): expose mnemosyne tool + write/show/link/graph + mirror"
```

---

## Task 5: 插件清单与 README

**Files:**
- Create: `mnemosyne/integrations/hermes/plugin.yaml`
- Create: `mnemosyne/integrations/hermes/README.md`

- [ ] **Step 1: 写 plugin.yaml**

```yaml
name: mnemosyne
version: 0.1.0
description: "Mnemosyne shared memory — read/write the same Mnemosyne store used by Claude Code and Codex, via the mnemosyne CLI bridge."
hooks:
  - on_memory_write
```

- [ ] **Step 2: 写 README.md**

```markdown
# Mnemosyne memory provider for Hermes

Bridges Hermes to the shared Mnemosyne store (the same one Claude Code and
Codex use), so long-term memory is shared across all three agents.

## Install

    python3 -m mnemosyne install-hermes

This copies the provider into `$HERMES_HOME/plugins/mnemosyne/` and sets
`memory.provider: mnemosyne` (plus a `plugins.mnemosyne` block) in
`config.yaml`. Restart Hermes to activate.

## Config (config.yaml)

    memory:
      provider: mnemosyne
    plugins:
      mnemosyne:
        python: /opt/homebrew/bin/python3   # a python that can `import mnemosyne`
        recall_limit: 5
        timeout: 5
        mirror_builtin_writes: false

## Behavior

- System prompt: injects global + project core memory (`mnemosyne read`).
- Each turn: recalls top matches (`mnemosyne search`).
- Tool `mnemosyne`: search / write / show / link / graph against the shared store.
- Writes are tagged `--source hermes[:<profile>]`.
```

- [ ] **Step 3: 提交**

```bash
git add mnemosyne/integrations/hermes/plugin.yaml mnemosyne/integrations/hermes/README.md
git commit -m "docs(hermes): add plugin manifest and provider README"
```

---

## Task 6: config 文本编辑纯函数

**Files:**
- Create: `mnemosyne/integrations/hermes/_install.py`
- Test: `tests/test_hermes_install.py`

- [ ] **Step 1: 写失败测试**

```python
# tests/test_hermes_install.py
from mnemosyne.integrations.hermes import _install


PY = "/opt/homebrew/bin/python3"


def test_set_provider_replaces_existing():
    src = "memory:\n  memory_enabled: true\n  provider: ''\n  nudge_interval: 10\nmodel:\n  default: x\n"
    out = _install.set_memory_provider(src)
    assert "  provider: mnemosyne\n" in out
    assert "  provider: ''" not in out
    assert "  memory_enabled: true\n" in out  # siblings preserved
    assert "model:\n  default: x\n" in out


def test_set_provider_inserts_when_absent():
    src = "memory:\n  memory_enabled: true\nmodel:\n  default: x\n"
    out = _install.set_memory_provider(src)
    assert "  provider: mnemosyne\n" in out


def test_set_provider_creates_block_when_no_memory():
    src = "model:\n  default: x\n"
    out = _install.set_memory_provider(src)
    assert "memory:\n  provider: mnemosyne\n" in out
    assert "model:\n  default: x\n" in out


def test_upsert_plugins_appends_block_when_absent():
    src = "memory:\n  provider: mnemosyne\n"
    out = _install.upsert_plugins_mnemosyne(src, PY)
    assert "plugins:\n  mnemosyne:\n" in out
    assert f"    python: {PY}\n" in out
    assert "    recall_limit: 5\n" in out


def test_upsert_plugins_into_existing_plugins_block():
    src = "plugins:\n  other:\n    foo: bar\nmemory:\n  provider: mnemosyne\n"
    out = _install.upsert_plugins_mnemosyne(src, PY)
    assert "  mnemosyne:\n" in out
    assert "  other:\n    foo: bar\n" in out  # sibling plugin preserved
    assert out.count("plugins:\n") == 1       # no duplicate top-level key


def test_upsert_plugins_is_idempotent():
    src = "memory:\n  provider: mnemosyne\n"
    once = _install.upsert_plugins_mnemosyne(src, PY)
    twice = _install.upsert_plugins_mnemosyne(once, PY)
    assert once == twice
```

- [ ] **Step 2: 运行确认失败**

Run: `python -m pytest tests/test_hermes_install.py -q`
Expected: FAIL（`ModuleNotFoundError: ... _install`）

- [ ] **Step 3: 实现纯函数**

```python
# mnemosyne/integrations/hermes/_install.py
"""install-hermes helpers: copy provider files + edit config.yaml (no PyYAML).

config.yaml is machine-managed (no comments/anchors), so targeted line edits
preserve all unrelated content byte-for-byte.
"""
from __future__ import annotations

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
    # find existing `  mnemosyne:` sub-block within plugins
    sub_start = None
    for k in range(start + 1, end):
        if lines[k].rstrip("\n") == "  mnemosyne:" or lines[k].startswith("  mnemosyne:"):
            sub_start = k
            break
    if sub_start is None:
        new = lines[: start + 1] + [block] + lines[start + 1:]
        return "".join(new)
    # replace existing sub-block (until next 2-space key or block end)
    sub_end = end
    for k in range(sub_start + 1, end):
        ln = lines[k]
        if ln.strip() and ln[:2] == "  " and ln[2:3] != " ":
            sub_end = k
            break
    new = lines[:sub_start] + [block] + lines[sub_end:]
    return "".join(new)
```

- [ ] **Step 4: 运行确认通过**

Run: `python -m pytest tests/test_hermes_install.py -q`
Expected: PASS（7 passed）

- [ ] **Step 5: 提交**

```bash
git add mnemosyne/integrations/hermes/_install.py tests/test_hermes_install.py
git commit -m "feat(hermes): config.yaml edit helpers (no PyYAML)"
```

---

## Task 7: `install-hermes` 安装编排 + CLI 接线

**Files:**
- Modify: `mnemosyne/integrations/hermes/_install.py`
- Modify: `mnemosyne/cli.py`
- Test: `tests/test_hermes_install.py`

- [ ] **Step 1: 写失败测试**

```python
import sys
from pathlib import Path


def test_install_copies_files_and_edits_config(tmp_path):
    hermes_home = tmp_path / ".hermes"
    hermes_home.mkdir()
    (hermes_home / "config.yaml").write_text(
        "memory:\n  memory_enabled: true\n  provider: ''\nmodel:\n  default: x\n",
        encoding="utf-8",
    )
    result = _install.install_hermes(
        hermes_home=hermes_home, python_path=sys.executable, force=False, write_config=True,
    )
    plugin_dir = hermes_home / "plugins" / "mnemosyne"
    assert (plugin_dir / "__init__.py").exists()
    assert (plugin_dir / "plugin.yaml").exists()
    cfg = (hermes_home / "config.yaml").read_text(encoding="utf-8")
    assert "provider: mnemosyne" in cfg
    assert "mnemosyne:" in cfg
    assert f"python: {sys.executable}" in cfg
    backups = list(hermes_home.glob("config.yaml.mnemosyne-bak-*"))
    assert len(backups) == 1
    assert result["config_written"] is True


def test_install_dry_run_writes_nothing(tmp_path):
    hermes_home = tmp_path / ".hermes"
    hermes_home.mkdir()
    (hermes_home / "config.yaml").write_text("memory:\n  provider: ''\n", encoding="utf-8")
    _install.install_hermes(hermes_home=hermes_home, python_path=sys.executable,
                            force=False, write_config=True, dry_run=True)
    assert not (hermes_home / "plugins" / "mnemosyne").exists()
    assert "provider: ''" in (hermes_home / "config.yaml").read_text(encoding="utf-8")


def test_install_no_config_skips_config(tmp_path):
    hermes_home = tmp_path / ".hermes"
    hermes_home.mkdir()
    (hermes_home / "config.yaml").write_text("memory:\n  provider: ''\n", encoding="utf-8")
    res = _install.install_hermes(hermes_home=hermes_home, python_path=sys.executable,
                                  force=False, write_config=False)
    assert (hermes_home / "plugins" / "mnemosyne" / "__init__.py").exists()
    assert "provider: ''" in (hermes_home / "config.yaml").read_text(encoding="utf-8")
    assert res["config_written"] is False
```

- [ ] **Step 2: 运行确认失败**

Run: `python -m pytest tests/test_hermes_install.py -k install -q`
Expected: FAIL（`_install.install_hermes` 不存在）

- [ ] **Step 3: 实现编排函数 + python 探测**

在 `_install.py` 末尾追加：

```python
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
                return str(Path(cand).resolve())
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
```

> 注：`detect_python` 与 provider 的 `_resolve_python` 探测顺序一致（explicit → which → homebrew → sys.executable），但返回绝对路径写进 config。

- [ ] **Step 4: 运行确认通过**

Run: `python -m pytest tests/test_hermes_install.py -q`
Expected: PASS（10 passed）

- [ ] **Step 5: 接线 CLI**

先看现有子命令注册方式：

Run: `grep -n "add_parser\|def main\|subparsers\|set_defaults\|args.command\|return 0\|elif args" mnemosyne/cli.py | head -50`

按既有 argparse 模式新增 `install-hermes` 子命令（下例为模式；以 cli.py 实际写法对齐——子解析器变量名、分派结构若不同，按文件内现有命令照搬，保持一致）：

```python
    p_inst = subparsers.add_parser("install-hermes",
                                   help="Install the Mnemosyne memory provider into Hermes")
    p_inst.add_argument("--python", default=None,
                        help="bridge python that can import mnemosyne")
    p_inst.add_argument("--hermes-home", default=None,
                        help="HERMES_HOME (default: $HERMES_HOME or ~/.hermes)")
    p_inst.add_argument("--force", action="store_true",
                        help="overwrite an existing plugin dir")
    p_inst.add_argument("--no-config", action="store_true",
                        help="do not edit config.yaml")
    p_inst.add_argument("--dry-run", action="store_true",
                        help="preview without writing")
```

分派处理（放进命令分派区，与其他命令同构）：

```python
    if args.command == "install-hermes":
        import os
        from pathlib import Path
        from mnemosyne.integrations.hermes import _install
        home = args.hermes_home or os.environ.get("HERMES_HOME") or str(Path.home() / ".hermes")
        try:
            result = _install.install_hermes(
                hermes_home=Path(home),
                python_path=args.python,
                force=args.force,
                write_config=not args.no_config,
                dry_run=args.dry_run,
            )
        except FileExistsError as exc:
            print(f"error: {exc}")
            return 1
        if args.dry_run:
            print("[dry-run] would install to", result["plugin_dir"])
            print("[dry-run] bridge python:", result["python"])
            if "config_preview" in result:
                print("[dry-run] config.yaml after edit:\n")
                print(result["config_preview"])
        else:
            print("Installed Mnemosyne provider to", result["plugin_dir"])
            print("Bridge python:", result["python"])
            if result["config_written"]:
                print("Updated config.yaml (backup:", result["backup"], ")")
                print("Restart Hermes to activate (memory.provider: mnemosyne).")
            else:
                print("Skipped config.yaml — set memory.provider: mnemosyne manually.")
        return 0
```

- [ ] **Step 6: 端到端冒烟（dry-run，不动真实 ~/.hermes）**

Run:
```bash
python -m mnemosyne install-hermes --hermes-home /tmp/hermes-smoke --python "$(command -v python3)" --dry-run
```
Expected: 打印 `[dry-run] would install to /tmp/hermes-smoke/plugins/mnemosyne`，并显示编辑后的 config 预览；`/tmp/hermes-smoke` 不被创建（dry-run）。

- [ ] **Step 7: 运行全部测试**

Run: `python -m pytest tests/test_hermes_install.py tests/test_hermes_provider.py -q`
Expected: PASS（全绿）

- [ ] **Step 8: 提交**

```bash
git add mnemosyne/integrations/hermes/_install.py mnemosyne/cli.py tests/test_hermes_install.py
git commit -m "feat(hermes): install-hermes CLI command"
```

---

## Task 8: 顶层 README Hermes 章节 + 全量回归

**Files:**
- Modify: `README.md`
- Test: 全量 `pytest`

- [ ] **Step 1: 在 README.md 增加 Hermes 接入小节**

定位现有讲 Codex/MCP 集成的章节，其后追加：

```markdown
### Hermes 接入

让 Hermes 与 Claude Code、Codex 共享同一个 Mnemosyne store：

    python3 -m mnemosyne install-hermes

把 provider 装进 `~/.hermes/plugins/mnemosyne/` 并在 `config.yaml` 设
`memory.provider: mnemosyne`。重启 Hermes 后，它会自动注入 core 记忆、每轮
检索，并通过 `mnemosyne` 工具写回共享 store。详见
`mnemosyne/integrations/hermes/README.md`。
```

- [ ] **Step 2: 全量回归**

Run: `python -m pytest -q`
Expected: 全绿（无新增失败；既有套件不受影响）

- [ ] **Step 3: 提交**

```bash
git add README.md
git commit -m "docs: document Hermes shared-memory integration"
```

---

## 部署（实现完成后，单独执行；非 git 交付）

> 这一步改动用户真实环境 `~/.hermes`，需用户在场确认后再执行。

- [ ] 真实安装：`python3 -m mnemosyne install-hermes --dry-run` 复核 → 去掉 `--dry-run` 正式装。
- [ ] 校验 bridge python：`/opt/homebrew/bin/python3 -c "import mnemosyne"` 退出 0。
- [ ] 重启 Hermes，确认日志出现 `Memory provider 'mnemosyne' activated`。
- [ ] 在 Hermes 里发一条会触发记忆的消息，确认 `## Mnemosyne recall` 注入与 `mnemosyne` 工具可用；用 `python3 -m mnemosyne search "<刚写的内容>"` 验证写回。

---

## Self-Review

**Spec coverage（对照 `docs/specs/2026-06-05-hermes-shared-memory-design.md`）：**
- §2.1 原生 provider → Task 1/3/4 ✓
- §2.2 子进程 CLI 调用 → Task 2 `_run` ✓
- §2.3 读 global+project（`read --scope all` / `search` 默认全 scope）→ Task 3 ✓；显式工具写入、无自动抽取（`sync_turn`/`on_session_end` no-op）✓；`--source hermes[:profile]` → Task 2/4 ✓；`mirror_builtin_writes` 默认关 → Task 4 ✓
- §3.1 代码归属 `mnemosyne/integrations/hermes/` → Task 1/5 ✓
- §3.2 `install-hermes` 拷文件 + 探测 python + 改 config + 备份 + `--dry-run`/`--no-config`/`--force` → Task 7 ✓
- §3.3 `register(ctx)` 入口 + 单文件 → Task 1 ✓
- §4.1 `_run` 失败降级 → Task 2 ✓
- §4.2 生命周期表 → Task 2/3/4 ✓
- §4.3 单 `mnemosyne` 工具 action 分派 → Task 4 ✓
- §4.4 config 由安装命令写入、python 绝对路径 → Task 6/7 ✓
- §5 错误降级 → Task 2/3/4（各方法失败返回空）✓
- §6 测试矩阵 → Task 2–7 全覆盖 ✓
- §7 交付物清单 → Task 1/5/7/8 ✓

**Placeholder scan：** 无 TBD/TODO；每个含代码的 step 均给出完整代码。CLI 接线（Task 7 Step 5）显式要求先 `grep` 对齐 cli.py 现有 argparse 写法——这是必要的代码库适配，非占位。

**Type consistency：** `MnemosyneMemoryProvider` 方法签名（`name` 为 property、`prefetch(query, *, session_id="")`、`handle_tool_call(tool_name, args, **kwargs)`、`on_memory_write(action, target, content, metadata=None)`）与 Hermes `MemoryProvider` ABC 一致；`_install.set_memory_provider` / `upsert_plugins_mnemosyne` / `detect_python` / `install_hermes` 在 Task 6/7 间签名一致；测试中 `_provider()`/`_seed_global()`/`_cli()` helper 命名贯穿一致。
