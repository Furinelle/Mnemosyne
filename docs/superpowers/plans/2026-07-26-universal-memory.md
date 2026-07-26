# Mnemosyne 通用化（通用内核 + 适配器）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Mnemosyne 的接入层重构为「通用内核 + 平级适配器」：稳定 Python API、四个中立注入事件、中立交接/transcript 格式、agent 无关的 CLI/MCP/文档表面（规格：`docs/superpowers/specs/2026-07-26-universal-memory-design.md`，P1–P4，PyPI 暂缓）。

**Architecture:** 新增 `api.py`（结构化结果）作为 CLI/MCP/inject 共用门面；`hooks/` 的通用逻辑上移为 `injection.py` + `session_state.py`，Claude Code hooks 迁入 `integrations/claude_code/` 成为第一个适配器；`codex.py` 拆为中立的 `findings.py` + `handoff.py` + `transcripts.py`。所有旧命令、旧模块路径、旧 MCP 工具名保留 alias/shim。

**Tech Stack:** Python 3.11+ stdlib + portalocker（不新增任何依赖）；pytest。

## Global Constraints

- 不新增运行时依赖；核心仍为 stdlib + `portalocker>=2.8`。
- 存储格式与 store 目录布局不变（Markdown + frontmatter、core.md、working/、archive/）。
- 兼容表面必须保留：`codex-prep`/`codex-ingest`/`install-hermes` CLI 名、`mnemosyne.hooks.*` 与 `mnemosyne.codex` 模块导入路径、MCP 工具 `mnemosyne_codex_prep`、`**新发现:**`/`**Findings:**` 交接块。
- hooks/适配器路径必须维持 fail-safe（异常时 exit 0、不污染 stdout 注入输出）。
- 每个 Task 结束时全量测试通过：`python3 -m pytest tests/ -q`。
- 提交信息用英文 conventional commits，正文可中文。

---

## Task 1: `findings.py` — 中立 Finding 模块（消除 distill→codex 反向依赖）

**Files:**
- Create: `mnemosyne/findings.py`
- Modify: `mnemosyne/codex.py`（Finding 改为 re-export）、`mnemosyne/distill/__init__.py:11`、`mnemosyne/distill/llm.py:10`
- Test: `tests/test_findings.py`

**Interfaces:**
- Produces: `mnemosyne.findings.Finding`（字段与现 `codex.Finding` 完全相同：`type/importance/title/tags/content/evidence`）；`mnemosyne.findings.allowed_types(config: dict | None = None) -> tuple[str, ...]`
- `allowed_types()`：有 config 时返回 `tuple(config["memory"]["types"])`，无 config 或空列表时回退硬编码 `('arch_decision','pitfall','codebase','preference','handoff','session_summary')`。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_findings.py
from mnemosyne.findings import Finding, allowed_types


def test_finding_fields():
    f = Finding(type="pitfall", importance=70, title="t", tags=["a"], content="c")
    assert f.evidence == ""


def test_allowed_types_fallback():
    assert "arch_decision" in allowed_types(None)
    assert "session_summary" in allowed_types({})


def test_allowed_types_from_config():
    cfg = {"memory": {"types": ["foo", "bar"]}}
    assert allowed_types(cfg) == ("foo", "bar")


def test_codex_reexport_is_same_class():
    from mnemosyne.codex import Finding as CodexFinding
    assert CodexFinding is Finding
```

- [ ] **Step 2: 运行确认失败**：`python3 -m pytest tests/test_findings.py -q` → ModuleNotFoundError
- [ ] **Step 3: 实现** `mnemosyne/findings.py`：

```python
"""Agent-neutral finding model shared by handoff, distill, and write paths."""

from __future__ import annotations

from dataclasses import dataclass

FALLBACK_TYPES = ('arch_decision', 'pitfall', 'codebase', 'preference', 'handoff', 'session_summary')


@dataclass
class Finding:
    type: str
    importance: int
    title: str
    tags: list[str]
    content: str
    evidence: str = ""


def allowed_types(config: dict | None = None) -> tuple[str, ...]:
    if config:
        types = config.get("memory", {}).get("types") or []
        if types:
            return tuple(str(t) for t in types)
    return FALLBACK_TYPES
```

codex.py：删除本地 `@dataclass Finding` 定义，改 `from mnemosyne.findings import Finding`；保留 `ALLOWED_TYPES = FALLBACK_TYPES`（导入自 findings）供旧代码引用。distill/`__init__.py` 与 distill/llm.py 的 `from mnemosyne.codex import Finding` 改为 `from mnemosyne.findings import Finding`（llm.py 的 ALLOWED_TYPES 本 task 先改 import 来源，动态化留给 Task 8）。
- [ ] **Step 4: 全量测试**：`python3 -m pytest tests/ -q` → PASS
- [ ] **Step 5: Commit** `refactor: extract neutral Finding model into findings.py`

---

## Task 2: `api.py` — 稳定结构化 API（write/search/maintain/link）

**Files:**
- Create: `mnemosyne/api.py`、`tests/conftest.py`
- Modify: `mnemosyne/cli.py`（`cmd_write` 核心事务抽入 api）、`mnemosyne/__init__.py`
- Test: `tests/test_api.py`

`tests/conftest.py`（全计划的测试共用，一次创建）：

```python
import pytest


@pytest.fixture
def tmp_store(tmp_path, monkeypatch):
    """Isolated global+project stores: HOME and cwd both point into tmp_path."""
    home = tmp_path / "home"
    project = tmp_path / "project"
    (project / ".git").mkdir(parents=True)
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))
    monkeypatch.chdir(project)
    from mnemosyne.store import Store, ensure_store, template_text
    store = Store("project", project / ".mnemosyne")
    ensure_store(store, template_text("core_project.md"))
    return store
```

**Interfaces:**
- Produces（后续 Task 3/5 依赖，签名固定）：

```python
@dataclass
class WriteResult:
    status: str            # "created" | "duplicate" | "superseded_old"
    id: str                # 新写入或命中的 memory id
    duplicate_of: str | None = None
    superseded: str | None = None
    path: str = ""

def write_entry(*, type: str, importance: int, content: str, title: str = "",
                tags: list[str] | None = None, scope: str = "project",
                source: str = "agent", expires: str = "",
                allow_duplicate: bool = False) -> WriteResult

def search_entries(query: str, *, scope: str = "all", type_filter: str = "",
                   limit: int = 5, include_archive: bool = False,
                   include_superseded: bool = False,
                   update_access: bool = True) -> list[dict]   # dict 字段 = 现 print_search_results output 项

def maintain(*, scope: str = "all", dry_run: bool = False,
             stores: list | None = None) -> dict  # {"processed":n,"decayed":n,"deprecated":n,"archived":n,"core_candidates":[{"id","summary"}]}

def link_entries(id1: str, id2: str, rel: str = "related",
                 allow_custom: bool = False, stores: list | None = None) -> dict  # {"ok": True, "rel": rel}
```

- `MnemosyneError(RuntimeError)`：未找到 memory、非法 relation 等抛它，CLI 转 stderr+exit code，MCP 转 JSON-RPC error。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_api.py（用 tests/helpers.py 的临时 store fixture 套路：monkeypatch HOME 与 cwd）
import pytest
from mnemosyne import api


def test_write_entry_created(tmp_store):          # tmp_store: 参照 tests/test_codex.py 现有 fixture 建临时项目 store
    result = api.write_entry(type="pitfall", importance=70, content="unique content alpha", title="T1")
    assert result.status == "created"
    assert result.id.startswith("pitfall-")


def test_write_entry_duplicate(tmp_store):
    first = api.write_entry(type="pitfall", importance=70, content="same body twice", title="T")
    second = api.write_entry(type="pitfall", importance=70, content="same body twice", title="T")
    assert second.status == "duplicate"
    assert second.duplicate_of == first.id


def test_link_entries_unknown_rel(tmp_store):
    a = api.write_entry(type="pitfall", importance=60, content="aaa body", title="A")
    b = api.write_entry(type="codebase", importance=60, content="bbb body", title="B")
    with pytest.raises(api.MnemosyneError):
        api.link_entries(a.id, b.id, rel="nonsense")


def test_maintain_dry_run(tmp_store):
    api.write_entry(type="pitfall", importance=70, content="ccc body", title="C")
    summary = api.maintain(dry_run=True)
    assert summary["processed"] >= 1
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**：
  - `api.write_entry` = 现 `cmd_write` 第 227-315 行中「非交互部分」的事务逻辑（ensure_store → 构造 Memory → `lock_store` 内 `classify_against_store` → duplicate 短路 / 写入 + `update_memory_index_file` + `update_search_index` + supersede 链接），原样搬移，返回 `WriteResult` 而非 print。
  - `cmd_write` 保留：参数解析、stdin 读 content、TTY 下 `duplicate_prompt` 交互、类型警告，然后调 `api.write_entry` 并按原文案 print（`Wrote {id}` / `Duplicate of {target}; skipped...` / `Supersedes {target}...`，保证 CLI 输出不变）。
  - `api.maintain` = 现 `cmd_maintain` 循环体（lock/maintain_memory/rewrite index/reindex），返回 dict；`cmd_maintain` 调它并按原格式 print。
  - `api.link_entries` = 现 `cmd_link` 事务体，找不到 memory / 非法 rel 抛 `MnemosyneError`；`cmd_link` 包装为原 stderr 文案 + exit code。
  - `api.search_entries` = `fusion_search` + 现 `print_search_results` 的 bump/字典组装（提取为共享函数 `_result_dicts`），`cmd_search` 复用。
  - `mnemosyne/__init__.py` 增加 `from mnemosyne import api`（惰性导入可用 `__getattr__`，避免 CLI 启动成本上升）。
- [ ] **Step 4: 全量测试** PASS（重点回归 `tests/test_codex.py`、`test_concurrency_v2.py`、`test_mcp_server.py`）
- [ ] **Step 5: Commit** `refactor: add stable structured api layer consumed by cli`

---

## Task 3: MCP 整改（去 stdout 解析、去 SDK 门槛、prep_context 改名、行为注解）

**Files:**
- Modify: `mnemosyne/mcp/server.py`、`mnemosyne/cli.py:626-633`（cmd_mcp_serve 不再捕获 MissingMCPDependency）、`README.md`（删 `mnemosyne[mcp]` 前置要求，本 task 只删这一处，整体重写在 Task 12）
- Test: `tests/test_mcp_server.py`（追加用例）

**Interfaces:**
- Consumes: Task 2 的 `api.write_entry / api.maintain / api.link_entries`
- Produces: MCP 工具 `mnemosyne_prep_context`（schema 与 `mnemosyne_codex_prep` 相同）；`mnemosyne_codex_prep` 保留为同 handler 的 alias；每个 TOOL_SCHEMAS 项增加 `annotations` 键。

- [ ] **Step 1: 写失败测试**（追加到 tests/test_mcp_server.py）

```python
def test_prep_context_tool_listed_with_alias():
    from mnemosyne.mcp.server import TOOL_HANDLERS, TOOL_SCHEMAS
    names = {schema["name"] for schema in TOOL_SCHEMAS}
    assert "mnemosyne_prep_context" in names
    assert "mnemosyne_codex_prep" not in names          # 列表只展示新名
    assert "mnemosyne_codex_prep" in TOOL_HANDLERS      # 旧名仍可调用


def test_tools_have_annotations():
    from mnemosyne.mcp.server import TOOL_SCHEMAS
    by_name = {schema["name"]: schema for schema in TOOL_SCHEMAS}
    assert by_name["mnemosyne_search"]["annotations"]["readOnlyHint"] is True
    assert by_name["mnemosyne_write"]["annotations"]["readOnlyHint"] is False


def test_serve_no_longer_requires_mcp_sdk(monkeypatch):
    import mnemosyne.mcp.server as server
    monkeypatch.delenv("MNEMOSYNE_MCP_ALLOW_STDLIB", raising=False)
    assert not hasattr(server, "_require_mcp_sdk")


def test_write_returns_structured_result(tmp_store):
    from mnemosyne.mcp.server import _write
    payload = _write({"type": "pitfall", "importance": 60, "content": "mcp body one", "title": "M"})
    assert payload["status"] == "created" and payload["id"].startswith("pitfall-")
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**：
  - `_write` → `api.write_entry(...)` 返回 `{"status": r.status, "id": r.id}`（duplicate 时 id=duplicate_of，保持现输出 shape）；`_maintain` → `api.maintain(...)`；`_link` → `api.link_entries(...)`；删除 `_capture_command`、`redirect_stdout` import。
  - `serve()` 删除 `_require_mcp_sdk` 调用与函数、`MNEMOSYNE_MCP_ALLOW_STDLIB` 检查；`MissingMCPDependency` 类保留但不再抛出（下个 minor 移除），`cmd_mcp_serve` 相应简化为直接 `return serve(sse=args.sse)`。
  - `mnemosyne_codex_prep` schema 项改名 `mnemosyne_prep_context`，description 改 "Assemble a memory context block (core + relevant memories) for any agent task."；`TOOL_HANDLERS` 同时注册两个名字指向 `_prep_context`（`_codex_prep` 改名）。
  - 每个 schema 加 `"annotations"`: search/read_core/show/graph → `{"readOnlyHint": True}`；write/link → `{"readOnlyHint": False, "destructiveHint": False, "idempotentHint": False}`；maintain → `{"readOnlyHint": False, "destructiveHint": True}`（会归档/衰减）；prep_context → `{"readOnlyHint": True}`。
  - doctor 的 mcp 检查行（cli.py:515-516）改为 `("mcp", True, "stdlib server built-in", False)`。
- [ ] **Step 4: 全量测试** PASS
- [ ] **Step 5: Commit** `refactor(mcp): call api layer, drop fake sdk gate, add prep_context alias + annotations`

---

## Task 4: `injection.py` + `session_state.py` — 通用注入逻辑上移

**Files:**
- Create: `mnemosyne/injection.py`、`mnemosyne/session_state.py`
- Modify: `mnemosyne/hooks/_common.py`（变成 re-export shim）、`mnemosyne/store.py:20-40`（DEFAULT_CONFIG 增加 injection.show_command_template 与 hooks.write_tools 默认值）
- Test: `tests/test_injection.py`

**Interfaces:**
- Produces:
  - `mnemosyne/injection.py`：`STOPWORDS`、`extract_keywords(text, limit=8)`、`collect_stores()`、`run_search(query, limit=5, update_access=False, stores=None)`、`format_for_injection(results, max_tokens=None, summary_chars=120, show_hint: str | None = DEFAULT_SHOW_HINT)`、`_approx_tokens`
  - `DEFAULT_SHOW_HINT = 'Run `python3 -m mnemosyne show <id>` for full detail.'`；`show_hint=None` 时不输出 footer；`resolve_show_hint(config, channel: str) -> str | None`：优先 `config["injection"].get("show_command_template")`（含 `{id}` 占位无需替换，整句作为 footer），`channel == "mcp"` 时返回 `'Call the mnemosyne_show tool with the id for full detail.'`，`channel == "none"` 返回 None。
  - `mnemosyne/session_state.py`：`load_injected_ids(session_id) / record_injected_ids(session_id, ids)`（逐行搬移现实现，含 TTL 清理）。
- 兼容：`mnemosyne/hooks/_common.py` 保留 `hook_safe`、`read_event`（Claude 协议专属，暂留原地），其余符号 `from mnemosyne.injection import ...` / `from mnemosyne.session_state import ...` re-export——`codex.py` 与四个 hook 脚本的既有 import 不需要改动即工作。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_injection.py
from mnemosyne.injection import extract_keywords, format_for_injection, resolve_show_hint


def test_extract_keywords_skips_stopwords():
    assert "认证" in extract_keywords("调试认证失败的问题")
    assert "the" not in extract_keywords("the quick brown fox")


def test_format_for_injection_no_footer():
    results = [{"id": "x-1", "scope": "project", "type": "pitfall", "tags": [], "summary": "s", "strength": 50, "score": 1.0}]
    text = format_for_injection(results, show_hint=None)
    assert "mnemosyne show" not in text and "x-1" in text


def test_resolve_show_hint_channels():
    assert "mnemosyne_show tool" in resolve_show_hint({}, "mcp")
    assert resolve_show_hint({}, "none") is None
    assert resolve_show_hint({"injection": {"show_command_template": "custom"}}, "cli") == "custom"


def test_common_shim_still_exports():
    from mnemosyne.hooks._common import extract_keywords as legacy
    assert legacy is extract_keywords
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**：逐行搬移 `_common.py` 的 38-48（STOPWORDS）、75-88、150-320 行进 injection.py（format_for_injection 的 footer 行改成 `if show_hint:` 受控）；91-147 行进 session_state.py；`_common.py` 只留 hook_safe/read_event + re-export。store.py DEFAULT_CONFIG：`"injection"` 节增加 `"show_command_template": ""`；新增 `"hooks": {"write_tools": ["Edit", "Write"]}` 节（DEFAULT_CONFIG_TOML 同步）。
- [ ] **Step 4: 全量测试** PASS（`tests/test_hooks.py` 必须不改即绿——证明 shim 有效）
- [ ] **Step 5: Commit** `refactor: lift generic injection + session-state logic out of claude hooks package`

---

## Task 5: `mnemosyne inject` — 通用注入事件 CLI

**Files:**
- Create: `mnemosyne/events.py`
- Modify: `mnemosyne/cli.py`（新增 inject 子命令）
- Test: `tests/test_events.py`

**Interfaces:**
- Consumes: Task 4 injection/session_state、Task 2 api、现有 distill。
- Produces: `mnemosyne/events.py`：

```python
@dataclass
class InjectionResult:
    context: str               # 可直接注入的文本；空串表示无内容
    memory_ids: list[str]
    approx_tokens: int

def handle_event(event: str, payload: dict, *, session: str = "",
                 channel: str = "cli", update_access: bool | None = None) -> InjectionResult
# event ∈ {"session_start", "turn_start", "file_touch", "session_end"}
# payload: session_start → {}；turn_start → {"prompt": str}
#          file_touch → {"files": [str, ...]}
#          session_end → {"text": str} 或 {"transcript": {"path": str, "format": "auto"}}
```

- 语义（与现 hooks 行为一一对应，保证适配器迁移后行为不变）：
  - `session_start`：拼 Global/Project Core（`### Global Core` / `### Project Core` 标题 + `## Mnemosyne Memory` 抬头），无 core 时 context=""。（auto-init 与 maintain 调度不在 events 内，留在适配器——它们是宿主策略。）
  - `turn_start`：prompt<10 字符返回空；extract_keywords → run_search(limit=3, update_access=True) → 会话去重 → format_for_injection（max_tokens/summary_chars 读 config，show_hint 按 channel）→ record_injected_ids。
  - `file_touch`：对 payload["files"] 每个取 basename，逐个 run_search(limit=2, update_access=False) 合并去重（保序），再走会话去重 + 格式化，标题 `## Memories relevant to {basename 列表逗号连接}`。
  - `session_end`：text 或 transcript（经 Task 9 的 parse_transcript，Task 9 之前先只支持 {"text"} 与 claude-jsonl path）→ `distill_text(..., commit=True)`（config distill.enabled 为假时返回空结果）→ context 为 `Mnemosyne: auto-saved memories:` 摘要行。
- CLI：`mnemosyne inject --event X [--session ID] [--channel cli|mcp|none] [--format text|json] [--fail-safe]`，payload 从 stdin 读 JSON（空 stdin 视为 `{}`）。`--format json` 输出 `{"context":...,"memory_ids":[...],"approx_tokens":n}`；`--fail-safe` 下任何异常打印空输出并 exit 0。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_events.py
import json
from mnemosyne import api
from mnemosyne.events import handle_event


def test_turn_start_injects_relevant_memory(tmp_store):
    api.write_entry(type="pitfall", importance=80, content="portalocker deadlock on windows", title="portalocker deadlock")
    result = handle_event("turn_start", {"prompt": "why does portalocker deadlock happen"}, session="s1")
    assert result.memory_ids and "portalocker" in result.context


def test_turn_start_dedups_by_session(tmp_store):
    api.write_entry(type="pitfall", importance=80, content="portalocker deadlock on windows", title="portalocker deadlock")
    first = handle_event("turn_start", {"prompt": "portalocker deadlock question"}, session="s2")
    second = handle_event("turn_start", {"prompt": "portalocker deadlock question"}, session="s2")
    assert first.memory_ids and not second.memory_ids


def test_file_touch_matches_basename(tmp_store):
    api.write_entry(type="codebase", importance=60, content="store.py handles locking", title="store.py notes", tags=["store.py"])
    result = handle_event("file_touch", {"files": ["/x/y/store.py"]}, session="s3")
    assert result.memory_ids


def test_unknown_event_raises(tmp_store):
    import pytest
    with pytest.raises(ValueError):
        handle_event("bogus", {})


def test_inject_cli_json(tmp_store, capsys, monkeypatch):
    import io, sys
    from mnemosyne.cli import main
    monkeypatch.setattr(sys, "stdin", io.StringIO(json.dumps({"prompt": "anything at all here"})))
    assert main(["inject", "--event", "turn_start", "--format", "json"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert set(payload) == {"context", "memory_ids", "approx_tokens"}
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现** events.py + cli `cmd_inject`（`--fail-safe` 用 try/except Exception: print("" or "{}"), return 0）
- [ ] **Step 4: 全量测试** PASS
- [ ] **Step 5: Commit** `feat: add agent-neutral injection event interface (mnemosyne inject)`

---

## Task 6: Claude Code hooks 迁入 `integrations/claude_code/`（协议壳 + shim）

**Files:**
- Create: `mnemosyne/integrations/claude_code/__init__.py`、`.../session_start.py`、`.../user_prompt_submit.py`、`.../pre_tool_use.py`、`.../stop.py`（从 `mnemosyne/hooks/` 平移）
- Modify: `mnemosyne/hooks/session_start.py`、`user_prompt_submit.py`、`pre_tool_use.py`、`stop.py` → 各缩为 shim；`mnemosyne/hooks/_common.py` 不动（Task 4 已是 shim）
- Test: `tests/test_hooks.py`（不改动原用例，追加 shim 断言）

**Interfaces:**
- Consumes: Task 5 `handle_event`；Task 4 `hook_safe/read_event`。
- Produces: 每个新模块保留 `main()`，内部改为「解析 Claude 事件 → handle_event → 包 hookSpecificOutput 输出」；`session_start` 保留 `maybe_auto_init_project`/`maybe_run_maintain`（属宿主策略，随适配器走）。

- [ ] **Step 1: 写失败测试**（追加）

```python
def test_hook_shims_delegate_to_integration():
    from mnemosyne.hooks import session_start as legacy
    from mnemosyne.integrations.claude_code import session_start as new
    assert legacy.main is new.main
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**：
  - 平移四个 hook 文件到 `integrations/claude_code/`，import 改指 `mnemosyne.injection` / `mnemosyne.session_state` / `mnemosyne.hooks._common`（hook_safe/read_event）。
  - `user_prompt_submit.main` 改为：读事件 → `handle_event("turn_start", {"prompt": prompt}, session=session_id, channel="cli")` → 有 context 才输出 `hookSpecificOutput`。`pre_tool_use.main`：`tool_name` 匹配改 `config.get("hooks", {}).get("write_tools", ["Edit", "Write"])`，兜底：tool_input 里有 `file_path` 字段也放行；然后 `handle_event("file_touch", {"files": [file_path]}, ...)`。`session_start.main` / `stop.main` 行为保持（stop 继续用 transcript_path + distill 路径，Task 9 后切到 parse_transcript）。
  - 旧 `mnemosyne/hooks/<name>.py` 改为三行 shim：

```python
from mnemosyne.integrations.claude_code.session_start import main

if __name__ == '__main__':
    main()
```

- [ ] **Step 4: 全量测试** PASS（test_hooks.py 原用例全绿 = 行为等价）
- [ ] **Step 5: Commit** `refactor: move claude code hooks into integrations/claude_code adapter`

---

## Task 7: `handoff.py` — codex.py 中立化改名 + 通道感知 prep

**Files:**
- Create: `mnemosyne/handoff.py`（prep/parse_findings/write_finding/ingest 从 codex.py 平移）
- Modify: `mnemosyne/codex.py` → re-export shim；`mnemosyne/distill/__init__.py:214,272`、`mnemosyne/mcp/server.py:18` 改 import handoff
- Test: `tests/test_codex.py`（不改原用例）+ 追加

**Interfaces:**
- Produces: `handoff.prep(task, max_memories=5, stores=None, channel="cli") -> str`：`channel="cli"` 输出现 CLI 提示（统一为 `python3 -m mnemosyne search ...`）；`channel="mcp"` 输出 "call the mnemosyne_search tool"；`channel="none"` 省略 "Mnemosyne CLI available" 段。其余函数签名不变。
- `mnemosyne/codex.py` 内容整体变为：

```python
"""Backward-compat shim; the neutral implementation lives in mnemosyne.handoff."""
from mnemosyne.findings import FALLBACK_TYPES as ALLOWED_TYPES, Finding  # noqa: F401
from mnemosyne.handoff import (  # noqa: F401
    CONTENT_OPEN_RE, FIELD_RE, FINDINGS_HEADER_RE,
    ingest, parse_findings, prep, write_finding,
)
```

- [ ] **Step 1: 写失败测试**（tests/test_handoff.py）

```python
from mnemosyne.handoff import prep


def test_prep_channel_mcp_mentions_tool(tmp_store):
    text = prep("some task", channel="mcp")
    assert "mnemosyne_search" in text and "python" not in text.lower()


def test_prep_channel_none_omits_cli_section(tmp_store):
    assert "CLI available" not in prep("some task", channel="none")


def test_codex_shim():
    import mnemosyne.codex as codex
    from mnemosyne import handoff
    assert codex.prep is handoff.prep and codex.parse_findings is handoff.parse_findings
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**（MCP `_prep_context` 传 `channel="mcp"`；cli `cmd_codex_prep` 传 `channel="cli"`；`api.py` 末尾补稳定别名 `prep_context = handoff.prep`、`ingest_findings = handoff.ingest`，凑齐 spec 4.1 的 API 面）
- [ ] **Step 4: 全量测试** PASS
- [ ] **Step 5: Commit** `refactor: rename codex handoff module to handoff.py with channel-aware prep`

---

## Task 8: findings 类型统一 + JSON 交接变体

**Files:**
- Modify: `mnemosyne/handoff.py`（parse_findings 接受 allowed；新增 parse_findings_json/parse_findings_auto；ingest 加 fmt 参数）、`mnemosyne/distill/llm.py`（动态类型）、`mnemosyne/cli.py`（ingest `--format`，Task 10 建的新命令此时还未存在则加在 codex-ingest 上）
- Create: `docs/handoff-format.md`（v1 Markdown 块 grammar + JSON schema，双语可后补，先英文）
- Test: `tests/test_handoff.py` 追加

**Interfaces:**
- Produces:
  - `parse_findings(text, allowed: tuple[str, ...] | None = None)`：None 时 `allowed_types(load_config())`。
  - `parse_findings_json(text, allowed=None) -> list[Finding]`：接受 `{"findings": [...]}` 或裸数组，字段同 Finding，非法项按现有 stderr 告警语气丢弃。
  - `parse_findings_auto(text, allowed=None)`：strip 后以 `{`/`[` 开头且能 json.loads → JSON 路径，否则 Markdown 路径。
  - `ingest(text, source='codex', commit=False, fmt='auto')`。
  - `distill/llm.py`：`LLMExtractor.__init__` 保存 `self._types = allowed_types(config)`，提示词与 `_parse_llm_json` 校验用它。

- [ ] **Step 1: 写失败测试**

```python
def test_parse_findings_json_roundtrip():
    from mnemosyne.handoff import parse_findings_auto
    text = '{"findings": [{"type": "pitfall", "importance": 66, "title": "T", "tags": ["a"], "content": "C"}]}'
    findings = parse_findings_auto(text)
    assert findings[0].type == "pitfall" and findings[0].importance == 66


def test_parse_findings_respects_custom_types():
    from mnemosyne.handoff import parse_findings
    block = "**Findings:**\n- type: custom_kind\n- importance: 60\n- title: T\n- tags: a\n- content: |\n    body\n"
    assert parse_findings(block, allowed=("custom_kind",))[0].type == "custom_kind"
    assert parse_findings(block, allowed=("pitfall",)) == []


def test_llm_prompt_uses_config_types(monkeypatch):
    from mnemosyne.distill.llm import LLMExtractor
    extractor = LLMExtractor({"memory": {"types": ["alpha", "beta"]}, "distill": {"llm": {}}})
    assert "alpha|beta" in extractor.prompt_preview()   # 暴露只读属性便于测试
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**（flush() 内 `ALLOWED_TYPES` 引用改为传入的 allowed；`prompt_preview()` 返回 `"|".join(self._types)` 所在的 base prompt）
- [ ] **Step 4: 全量测试** PASS
- [ ] **Step 5: Commit** `feat: config-driven finding types + json handoff variant`

---

## Task 9: `transcripts.py` — 解析器注册表 + distill 中立化

**Files:**
- Create: `mnemosyne/transcripts.py`
- Modify: `mnemosyne/distill/__init__.py`（parse_claude_transcript 迁移并 re-export；distill_text source 默认改 "agent"）、`mnemosyne/cli.py`（distill `--format`、`--source` 默认 "agent"、help 文案中立）、`mnemosyne/integrations/claude_code/stop.py`（显式传 source="claude-code"、format="claude-jsonl"）
- Test: `tests/test_transcripts.py`

**Interfaces:**
- Produces:

```python
# mnemosyne/transcripts.py
PARSERS = {"claude-jsonl": ..., "role-jsonl": ..., "text": ...}

def parse_transcript(path_or_text: Path | str, fmt: str = "auto") -> list[Turn]
def detect_format(sample: str) -> str
# role-jsonl：每行 {"role": "user"|"assistant", "text": "..."}；其他 role 忽略
# detect: 首个非空行是 JSON 且含 "message" 键 → claude-jsonl；JSON 且含 "role"+"text" → role-jsonl；否则 text（走 _parse_role_lines 再兜底整段 assistant）
```

- `Turn` dataclass 从 distill 迁到 transcripts（distill re-export 保兼容）。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_transcripts.py
import json
from mnemosyne.transcripts import detect_format, parse_transcript


def test_role_jsonl(tmp_path):
    p = tmp_path / "t.jsonl"
    p.write_text('\n'.join(json.dumps(x) for x in [
        {"role": "user", "text": "如何修复认证失败"},
        {"role": "assistant", "text": "根因是 token 过期"},
    ]), encoding="utf-8")
    turns = parse_transcript(p)
    assert [t.role for t in turns] == ["user", "assistant"]


def test_detect_claude_jsonl():
    line = json.dumps({"message": {"role": "user", "content": "hi"}})
    assert detect_format(line) == "claude-jsonl"


def test_detect_plain_text():
    assert detect_format("[user] hello\n[assistant] hi") == "text"


def test_distill_source_default_neutral(tmp_store, monkeypatch):
    import io, sys
    from mnemosyne.cli import build_parser
    args = build_parser().parse_args(["distill", "--stdin"])
    assert args.source == "agent"
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**（`cmd_distill`：`--transcript` 时 `parse_transcript(args.transcript, args.format)`；distill/__init__ 顶部 `from mnemosyne.transcripts import Turn, parse_transcript` 并保留 `parse_claude_transcript = lambda p: parse_transcript(p, "claude-jsonl")` 形式的兼容函数——用 def 不用 lambda）
- [ ] **Step 4: 全量测试** PASS（test_distill.py 不改即绿）
- [ ] **Step 5: Commit** `feat: transcript parser registry with auto-detection; neutral distill defaults`

---

## Task 10: CLI 表面 — prep/ingest/install 通用化 + init --agent

**Files:**
- Modify: `mnemosyne/cli.py`（build_parser + cmd_init）
- Create: `mnemosyne/integrations/_registry.py`（`INSTALLERS = {"hermes": ..., "claude-code": ...}`）
- Test: `tests/test_cli_surface.py`

**Interfaces:**
- Produces:
  - `prep` / `ingest` 子命令（参数同 codex-prep/codex-ingest + ingest 增 `--format`）；`codex-prep`/`codex-ingest` 保留，help 标注 "(alias of prep/ingest)"。实现方式：两组 parser 共用同一 `cmd_prep`/`cmd_ingest` 函数。
  - `install` 子命令：`mnemosyne install <agent>`，agent 从 `_registry.INSTALLERS` 取；`hermes` 移植现 cmd_install_hermes 逻辑；`claude-code` 打印合并 hooks 的说明并输出 settings.json 路径（v1 不自动改 `~/.claude/settings.json`，只指导；自动合并留待后续）。`install-hermes` 保留 alias。
  - `init`：`--agent`（choices 取 registry keys + "codex"）与 `--no-agent-files`。默认写 generic `AGENTS.md`（Task 11 的 generic 模板）；`--agent codex` 写 codex 模板；`--agent claude-code` 额外打印 hooks 指引。
- Consumes: Task 11 的模板路径（本 task 先引用 `template_text("agents/generic/AGENTS.md")`，Task 11 创建；两个 task 同一次提交序列内完成，顺序：先 Task 11 后 Task 10 亦可——执行时若模板缺失先做 Task 11）。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_cli_surface.py
from mnemosyne.cli import build_parser


def test_prep_and_alias_share_handler():
    parser = build_parser()
    prep_args = parser.parse_args(["prep", "task"])
    legacy_args = parser.parse_args(["codex-prep", "task"])
    assert prep_args.func is legacy_args.func


def test_ingest_has_format_flag():
    args = build_parser().parse_args(["ingest", "--format", "json"])
    assert args.fmt == "json"


def test_install_hermes_alias():
    parser = build_parser()
    new = parser.parse_args(["install", "hermes"])
    legacy = parser.parse_args(["install-hermes"])
    assert new.agent == "hermes" and hasattr(legacy, "func")


def test_init_no_agent_files(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    from mnemosyne.cli import main
    assert main(["init", "--no-agent-files"]) == 0
    assert not (tmp_path / "AGENTS.md").exists()
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**（注意 argparse alias 用 `subparsers.add_parser("codex-prep", ...)` 独立注册但 set_defaults 同函数；ingest 的 `--format` 存入 `dest="fmt"` 防与 distill 冲突；`write`/`ingest`/`distill` 的 `--source` 入口统一做 `value.strip().lower()` 归一化，语法 `<agent>[:<profile>]`——正则 `^[a-z0-9_-]+(:[a-z0-9_.-]+)?$` 不匹配时仅 stderr 警告不阻断）
- [ ] **Step 4: 全量测试** PASS
- [ ] **Step 5: Commit** `feat(cli): neutral prep/ingest/install commands with legacy aliases; init --agent`

---

## Task 11: 模板重排 — templates/agents/<agent>/

**Files:**
- Create: `mnemosyne/templates/agents/generic/AGENTS.md`（中立版：三条接入路径 + findings 块说明，英文）、`mnemosyne/templates/agents/codex/AGENTS.md`（现模板平移）、`mnemosyne/templates/agents/claude_code/CLAUDE.md`（现模板平移）、`mnemosyne/templates/agents/claude_code/settings.json`（现模板平移）
- Modify: `mnemosyne/store.py` 的 `template_text()`（支持子路径 `agents/generic/AGENTS.md`；顶层旧名继续可用）；旧 `templates/AGENTS.md`、`CLAUDE.md`、`settings.json` 保留原样（hooks/文档仍引用）
- Test: `tests/test_templates.py`

**Interfaces:**
- Produces: `template_text("agents/generic/AGENTS.md")` 可用；generic 模板内容要点（英文）：memory store 是 Markdown 文件可直接读写；`python3 -m mnemosyne search/write` 用法；MCP server 一行启动；findings 块（Markdown v1 + JSON 变体）示例。

- [ ] **Step 1: 写失败测试**

```python
# tests/test_templates.py
from mnemosyne.store import template_text


def test_generic_agents_template():
    text = template_text("agents/generic/AGENTS.md")
    assert "mnemosyne search" in text and "Findings" in text


def test_legacy_template_names_still_work():
    assert template_text("AGENTS.md")
    assert template_text("core_project.md")
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**（template_text 现实现如果是 `(templates_dir() / name).read_text` 形式则天然支持子路径，只需建目录与文件；确认 pyproject 的 package-data 含 `templates/agents/**`）
- [ ] **Step 4: 全量测试** PASS
- [ ] **Step 5: Commit** `feat: per-agent template directories with neutral generic AGENTS.md`

---

## Task 12: 文档重定位 — 英文 README + README.zh.md + adapters/interface 文档

**Files:**
- Create: `README.zh.md`（现 README.md 全文迁移 + 叙事更新）、`docs/adapters.md`、`docs/interface.md`
- Modify: `README.md`（重写为英文）、`pyproject.toml`（keywords）、`CHANGELOG.md`（0.7.0 条目）、`mnemosyne/__init__.py`（`__version__ = "0.7.0"`）
- Test: `tests/test_version.py`（版本断言更新）

**Interfaces / 内容要求:**
- `README.md`（英文，结构）：定位段（"local-first, agent-agnostic memory kernel — Markdown files as source of truth, SQLite FTS5 hybrid retrieval, zero heavy dependencies"）→ Why（对比重基础设施方案）→ Quickstart → **Integrate any agent** 三小节（1. MCP：`python3 -m mnemosyne mcp serve` + 客户端配置片段指引；2. CLI/inject：四事件表 + `mnemosyne inject` 示例；3. Files：直接读写 `.mnemosyne/` 的格式指引，链接 docs/interface.md）→ Official adapters（Claude Code / Codex / Hermes 各一段 + install 命令）→ 功能表（自现 README 精简翻译）→ eval/benchmark 段 → 链接 README.zh.md。
- `README.zh.md`：现中文 README 全文，首段与功能表按新定位改写（「通用记忆内核 + 官方适配器」），集成章节与英文版对齐。
- `docs/adapters.md`（英文）：适配器契约——四事件的 payload schema、调用时机、fail-safe 约定、session key 语义、transcript 供给（role-jsonl）、`install` 注册方式、契约测试跑法；Claude Code 与 Hermes 作为参考实现讲解。**注意补充 Hermes 已知事项：外接 provider 不会自动停用宿主内建记忆，需在宿主配置中显式关闭（memory.provider: mnemosyne）。**
- `docs/interface.md`（英文）：frontmatter 字段表（含手写 YAML 解析器支持子集的警示）、`search --format json` 输出字段表、config.toml 键表（含信任边界说明摘引）、source 命名规范 `<agent>[:<profile>]`、findings 格式链接 docs/handoff-format.md。
- `pyproject.toml` keywords → `["agent-memory", "memory", "mcp", "llm", "claude-code", "codex"]`。

- [ ] **Step 1:** 更新 `tests/test_version.py` 断言 0.7.0，运行确认失败
- [ ] **Step 2:** bump `__version__`，写四份文档与 CHANGELOG，测试通过
- [ ] **Step 3:** 全量测试 PASS
- [ ] **Step 4: Commit** `docs: reposition as agent-agnostic memory kernel (en README + zh + adapter/interface specs); v0.7.0`

---

## Task 13: 适配器契约测试 + 兼容回归套件

**Files:**
- Create: `tests/test_adapter_contract.py`、`tests/test_compat_aliases.py`
- Test: 本 task 即测试

**Interfaces:**
- Consumes: Task 5 handle_event、Task 6 适配器、Task 7/8 handoff、Task 9 transcripts。

- [ ] **Step 1: 写契约测试**

```python
# tests/test_adapter_contract.py — 任何新适配器改 EVENT_CASES 即可复用
import json
from mnemosyne import api
from mnemosyne.events import handle_event

EVENTS = ["session_start", "turn_start", "file_touch", "session_end"]


def test_all_events_return_injection_result(tmp_store):
    payloads = {
        "session_start": {},
        "turn_start": {"prompt": "a prompt that is long enough"},
        "file_touch": {"files": ["a.py"]},
        "session_end": {"text": "[user] hi\n[assistant] done"},
    }
    for event in EVENTS:
        result = handle_event(event, payloads[event], session="contract")
        assert isinstance(result.context, str) and isinstance(result.memory_ids, list)


def test_findings_roundtrip_markdown_and_json(tmp_store):
    from mnemosyne.handoff import ingest
    md = "**Findings:**\n- type: pitfall\n- importance: 60\n- title: T\n- tags: a\n- content: |\n    body md\n"
    js = json.dumps({"findings": [{"type": "pitfall", "importance": 60, "title": "T2", "tags": [], "content": "body js"}]})
    assert ingest(md, commit=False)[0]["title"] == "T"
    assert ingest(js, commit=False, fmt="auto")[0]["title"] == "T2"


def test_distill_idempotent(tmp_store):
    from mnemosyne.distill import distill_text
    text = "[user] 记住：以后都用 uv 不要用 pip\n[assistant] 好的"
    first = distill_text(text, source="agent", commit=True)
    second = distill_text(text, source="agent", commit=True)
    assert all(a["verdict"] == "duplicate" for a in second) or not second
```

```python
# tests/test_compat_aliases.py
def test_legacy_imports():
    from mnemosyne.codex import Finding, prep, parse_findings, ingest, write_finding  # noqa
    from mnemosyne.hooks._common import extract_keywords, run_search, format_for_injection  # noqa
    from mnemosyne.hooks.session_start import main  # noqa
    from mnemosyne.distill import parse_claude_transcript, Turn  # noqa


def test_legacy_cli_names():
    from mnemosyne.cli import build_parser
    parser = build_parser()
    for argv in (["codex-prep", "t"], ["codex-ingest"], ["install-hermes"]):
        assert hasattr(parser.parse_args(argv), "func")


def test_legacy_mcp_tool_name():
    from mnemosyne.mcp.server import TOOL_HANDLERS
    assert "mnemosyne_codex_prep" in TOOL_HANDLERS
```

- [ ] **Step 2:** 运行，修复暴露的问题直至全绿
- [ ] **Step 3:** 全量测试 + `python3 -m mnemosyne doctor` 手工冒烟
- [ ] **Step 4: Commit** `test: adapter contract suite + backward-compat regression tests`

---

## Task 14: Hermes 适配器中立化 + 通用 CLIBridge（spec 4.3 收尾）

**Files:**
- Create: `mnemosyne/integrations/_bridge.py`
- Modify: `mnemosyne/integrations/hermes/__init__.py`（工具描述、python 候选路径、改用 CLIBridge）
- Test: `tests/test_hermes_provider.py`（原用例不改，追加）

**Interfaces:**
- Produces: `CLIBridge(python_candidates: list[str] | None = None, timeout: float = 10.0)`，方法 `run(args: list[str], stdin: str | None = None) -> tuple[int, str, str]`（returncode/stdout/stderr）与 `run_json(args, stdin=None) -> object | None`（解析失败/超时返回 None）。逻辑 = 现 hermes `__init__.py` 中 `_run` / `_python_has_mnemosyne` / JSON 解析 / 超时降级的平移；python 解释器候选列表参数化（默认保持现值含 `/opt/homebrew/bin/python3`，但允许 config/环境变量 `MNEMOSYNE_BRIDGE_PYTHON` 覆盖）。
- Hermes `MNEMOSYNE_TOOL` 描述文案改为 "Shared long-term memory for all agents via Mnemosyne (search before asking the user to repeat context)."。

- [ ] **Step 1: 写失败测试**

```python
def test_clibridge_run_json_roundtrip():
    from mnemosyne.integrations._bridge import CLIBridge
    bridge = CLIBridge()
    payload = bridge.run_json(["-c", "import json; print(json.dumps({'ok': 1}))"], raw_python=True)
    assert payload == {"ok": 1}


def test_bridge_python_env_override(monkeypatch):
    from mnemosyne.integrations._bridge import CLIBridge
    monkeypatch.setenv("MNEMOSYNE_BRIDGE_PYTHON", "/nonexistent/python3")
    assert CLIBridge().candidates[0] == "/nonexistent/python3"


def test_hermes_tool_description_neutral():
    from mnemosyne.integrations.hermes import MNEMOSYNE_TOOL
    text = str(MNEMOSYNE_TOOL)
    assert "Claude Code, Codex" not in text
```

（`raw_python=True` 表示直接以候选解释器执行给定 argv，不自动前缀 `-m mnemosyne`；默认 False 时自动前缀。）

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 全量测试** PASS（`test_hermes_provider.py`、`test_hermes_install.py` 原用例全绿）
- [ ] **Step 5: Commit** `refactor(hermes): extract reusable CLIBridge, neutral tool description, configurable python`

---

## 执行顺序与验收

顺序：1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 11 → 10 → 14 → 12 → 13（Task 11 先于 10，因 init --agent 引用 generic 模板；Task 12 收尾文档需引用 14 的结果）。

最终验收：
1. `python3 -m pytest tests/ -q` 全绿；
2. `python3 -m mnemosyne doctor` 无 hard failure；
3. `echo '{"prompt":"portalocker deadlock"}' | python3 -m mnemosyne inject --event turn_start --format json` 输出合法 JSON；
4. 现有 Claude Code hooks 配置（`~/.claude/settings.json` 引用 `mnemosyne.hooks.*`）不改动仍工作；
5. README.md 为英文、README.zh.md 存在且互链。
