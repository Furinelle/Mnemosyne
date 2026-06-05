# Hermes 共享 Mnemosyne 记忆 — 设计文档

- **日期**: 2026-06-05
- **状态**: 已批准，待实现
- **目标**: 让 Hermes（`~/.hermes` 桌面/网关 agent）与 Claude Code、Codex 读写**同一个** Mnemosyne 全局 store，实现三端长期记忆共享。

## 1. 背景

Mnemosyne 当前作为 Claude Code 与 Codex 之间的共享记忆桥：两者都通过 `python3 -m mnemosyne` CLI 操作同一个全局 store（`~/.mnemosyne/`）。Claude 端走 hooks（`mnemosyne/hooks/`），Codex 端走 AGENTS.md 指令 + `mnemosyne/codex.py`。

Hermes（`~/.hermes/hermes-agent`，Python 3.11，uv venv）此前不接入。调研发现 Hermes 自带一套**可插拔 `MemoryProvider` ABC**（`agent/memory_provider.py`）：外部后端（Honcho、Mem0、Hindsight、holographic 等）以 `plugins/memory/<name>/` 形式提供，通过 `config.yaml` 的 `memory.provider` 激活。其生命周期 hook 与 Mnemosyne 原语几乎一一对应，是理想的接入面。

当前状态确认：`~/.hermes/memories/` 为空，`memory.provider: ''`，尚无 provider 激活。

## 2. 关键决策

### 2.1 集成方式：原生 MemoryProvider 插件（非 MCP、非 CLI 指令）
- **选择**：写一个 Hermes 原生 `MemoryProvider` 插件包住 Mnemosyne。
- **理由**：只有 provider 路径能拿到 Hermes 的自动注入生命周期（`system_prompt_block` 注入 core、`prefetch` 每轮检索），体验最接近 Claude 的 hooks。
- **否选 MCP**：MCP 只给模型工具，无自动注入，模型得自己想起来查。
- **否选 AGENTS.md 式 CLI 指令**：零代码但全靠模型自觉，无自动注入。

### 2.2 调用机制：子进程调 CLI（非进程内 import）
- **选择**：provider shell out 到 `python3 -m mnemosyne <cmd> --format json`。
- **理由**：与现有 Claude/Codex 的 bridge 契约一致；零依赖污染 Hermes venv；进程隔离（Mnemosyne 出错不拖垮 Hermes）；Mnemosyne 可独立演进，CLI JSON 即接口。
- **否选进程内 import**：需给 Hermes venv 装 `portalocker`（当前没有）+ 注入 repo 路径到 `sys.path`，且耦合 repo 位置、Mnemosyne 崩溃会波及 Hermes 进程。
- **代价**：每轮 `prefetch` ~100–300ms 子进程延迟，用 Hermes 异步 `queue_prefetch` 掩盖。

### 2.3 四个行为默认值
- **读取范围**：global + project（CLI 默认 / `--scope all`）。Hermes 在非项目目录运行时只有 global —— 即真正的跨 agent 共享层。
- **写入策略**：仅显式工具调用，**不做每轮正则自动抽取**。因为 Claude/Codex 也读这个共享 store，自动抽取会污染它。
- **来源标记**：所有写入带 `--source hermes`（带 profile 时为 `hermes:<profile>`）。
- **不镜像 Hermes 内置记忆**：`on_memory_write` 默认关，避免与内置 provider 重复写入。配置可开。

## 3. 架构

### 3.1 代码归属
Mnemosyne 已"拥有"它对接各 agent 的适配层（`hooks/`、`codex.py`、`mcp/`），Hermes 适配层同理归入**本仓库**：

```
mnemosyne/integrations/
  __init__.py
  hermes/
    __init__.py        # provider 模块（被拷到 ~/.hermes/plugins/mnemosyne/__init__.py）
    plugin.yaml        # Hermes 插件清单
    README.md          # 安装/配置说明
```

源码进 git、可测、可复现，而非裸写进 `~/.hermes`。

### 3.2 安装/部署
新增 CLI 子命令 `python3 -m mnemosyne install-hermes [--python PATH] [--hermes-home PATH] [--force] [--no-config] [--dry-run]`：

1. 解析 `$HERMES_HOME`（默认 `~/.hermes`，可 `--hermes-home` 覆盖）。
2. 把 `integrations/hermes/{__init__.py, plugin.yaml, README.md}` 拷到 `$HERMES_HOME/plugins/mnemosyne/`（用户级插件目录，随 Hermes 更新不被覆盖；已存在则需 `--force`）。
3. 探测 bridge python（`--python` 显式指定，否则找一个能 `import mnemosyne` 的 python：依次试 `shutil.which("python3")`、`/opt/homebrew/bin/python3`、`sys.executable`），解析为绝对路径。
4. **直接改写 `$HERMES_HOME/config.yaml`**（除非 `--no-config`），用定向文本编辑、改前备份：
   - 先备份到 `config.yaml.mnemosyne-bak-<YYYYMMDDHHMMSS>`。
   - 设 `memory.provider: mnemosyne`（替换 `memory:` 块下既有的 `provider:` 行；该行若不存在则在 `memory:` 块内插入；`memory:` 块若不存在则在文件末尾补建）。
   - upsert 顶层 `plugins.mnemosyne` 块（含探测到的 python 绝对路径与默认值）：若已有顶层 `plugins:` 则在其下插入/更新 `mnemosyne:` 子块，否则在文件末尾追加整个 `plugins:` 块。
   - **幂等**：重复运行只更新对应键，不重复追加。
   - `--dry-run`：只打印将做的 config 改动 diff，不落盘。
5. 打印激活结果与校验提示（bridge python 路径、如何 `is_available()` 自检）。

> **为何用文本编辑而非 PyYAML**：bridge python 无 PyYAML，且 Mnemosyne 全程不依赖 PyYAML。已验证 `~/.hermes/config.yaml` 为机器管理（0 注释、2 空格缩进、无锚点/多行标量），定向行级编辑安全且可保留其余内容逐字不变。

### 3.3 Hermes 插件加载路径（已验证）
- Hermes `plugins/memory/__init__.py:load_memory_provider(name)` 在 `$HERMES_HOME/plugins/<name>/` 找到目录后，按 `register(ctx)` 入口优先、否则实例化 `MemoryProvider` 子类。
- `agent/agent_init.py` 读 `memory.provider` → `load_memory_provider` → `initialize_all(**kwargs)`，kwargs 含 `session_id, platform, hermes_home, agent_identity(profile), user_id, user_name, chat_id` 等。
- provider 的 `get_tool_schemas()` 会被注入工具面。
- **约束**：整个 provider 保持单文件 `__init__.py`，避免相对导入在 Hermes 合成命名空间（`_hermes_user_memory.mnemosyne`）下的边角问题。

## 4. 组件设计

### 4.1 子进程桥接 `_run`
```
_run(args: list[str], *, json_out: bool=False, timeout: float=5.0) -> str | dict | list
```
- 命令：`[self._python, "-m", "mnemosyne", *args]`
- `cwd`：Hermes 运行时 cwd（`os.getcwd()`，使 project store 按当前目录解析）
- `env`：继承当前环境
- 失败处理：非零退出 / 超时 / JSON 解析失败 → 返回空（`""` 或 `[]`/`{}`）。**永不抛出**到 Hermes 调用方。
- 日志：失败走 `logger.debug`，不刷屏。

### 4.2 `MnemosyneMemoryProvider(MemoryProvider)`

| 方法 | 实现 |
|---|---|
| `name` | `"mnemosyne"` |
| `is_available()` | `_run(["read", "--scope", "global"])` 非空或 exit 0 → True；否则 False |
| `initialize(session_id, **kw)` | 存 `session_id`、`agent_identity`（→ source 后缀）、读插件 config（`python`/`recall_limit`/`timeout`/`mirror_builtin_writes`）；解析 bridge python |
| `system_prompt_block()` | `_run(["read", "--scope", "all"])` → 包一层 `# Mnemosyne Shared Memory\n...`；空则返回 `""` |
| `prefetch(query, *, session_id="")` | 空 query 返回 `""`；否则 `_run(["search", query, "--format", "json", "--limit", str(recall_limit)], json_out=True)` → 格式化 `## Mnemosyne recall\n- [score] title — content摘要` |
| `sync_turn(...)` | no-op（写入只走显式工具） |
| `get_tool_schemas()` | 返回单个 `mnemosyne` 工具（见 4.3） |
| `handle_tool_call("mnemosyne", args)` | 按 `action` 分派到 CLI 子命令，返回 JSON 字符串 |
| `on_session_end(messages)` | no-op（默认） |
| `on_memory_write(action, target, content)` | 仅当 `mirror_builtin_writes=True` 时，把内置写入镜像成一条 `--source hermes` 的 global preference/codebase |
| `shutdown()` | no-op |

### 4.3 暴露给模型的工具（单工具 action 分派）

```json
{
  "name": "mnemosyne",
  "description": "Shared long-term memory across Claude Code, Codex, and Hermes. Search past memories, or write durable facts (preferences, decisions, pitfalls, codebase knowledge) the user would expect remembered across sessions and agents.",
  "parameters": {
    "type": "object",
    "properties": {
      "action": {"type": "string", "enum": ["search", "write", "show", "link", "graph"]},
      "query":  {"type": "string", "description": "search query (action=search)"},
      "limit":  {"type": "integer", "description": "max results (action=search, default 5)"},
      "type":   {"type": "string", "description": "memory type (action=write): preference|arch_decision|pitfall|codebase|handoff"},
      "importance": {"type": "integer", "description": "50-90 (action=write)"},
      "scope":  {"type": "string", "enum": ["global", "project"], "description": "write scope (action=write, default global)"},
      "title":  {"type": "string"},
      "content":{"type": "string"},
      "tags":   {"type": "string", "description": "comma-separated"},
      "id":     {"type": "string", "description": "memory id (action=show|graph|link)"},
      "id2":    {"type": "string", "description": "second id (action=link)"},
      "rel":    {"type": "string", "description": "relation (action=link): caused_by|refines|supersedes|contradicts|related"}
    },
    "required": ["action"]
  }
}
```

action → CLI 映射：
- `search` → `search <query> --format json --limit <limit>`
- `write`  → `write --type <type> --importance <importance> --scope <scope> --source hermes[:<profile>] --title <title> --content <content> --tags <tags> --force`
- `show`   → `show <id>`
- `link`   → `link <id> <id2> --rel <rel>`
- `graph`  → `graph <id> --format mermaid`

### 4.4 配置（`~/.hermes/config.yaml`，由 `install-hermes` 自动写入）
```yaml
memory:
  provider: mnemosyne
plugins:
  mnemosyne:
    python: /opt/homebrew/bin/python3   # 安装时探测到的 bridge python 绝对路径
    recall_limit: 5
    timeout: 5
    mirror_builtin_writes: false
```
- `python` 由安装命令探测后写入绝对路径（非写死），用户可后续手改。
- 插件 config 通过 `_load_plugin_config()` 读 `plugins.mnemosyne`（参照 holographic）。

## 5. 错误处理与降级

- 所有 CLI 调用经 `_run` 包裹，失败一律返回空 → Hermes 端表现为"本轮无记忆注入/工具返回空结果"，agent 正常继续。
- `is_available()` False 时，`agent_init` 不激活 provider，Hermes 退回内置记忆，无副作用。
- bridge python 找不到 mnemosyne → `is_available()` False，安装命令应在打印配置时提示用户校验 python 路径。

## 6. 测试（`tests/test_hermes_provider.py`）

provider 单文件可被 `tests/` 直接 import（通过 `mnemosyne.integrations.hermes`）。用临时 `$HOME` + 临时全局 store：

1. `is_available()`：store 就绪 → True；伪造坏 python 路径 → False。
2. `system_prompt_block()`：写入一条 core 后，返回含该 core 文本。
3. `prefetch(query)`：预置可命中的 memory，返回含其 title 的格式化块；无命中 → `""`。
4. `handle_tool_call` 往返：`action=write` 写入 → `action=search` 能搜到。
5. 降级：bridge 命令故意失败（超时/非零）→ 各方法返回空、不抛异常。
6. `install-hermes` CLI（用临时 `$HERMES_HOME`）：
   - 拷文件到 `$HERMES_HOME/plugins/mnemosyne/`；已存在时 `--force` 行为。
   - **config 写入**：空/最简 config → 正确生成 `memory.provider` 与 `plugins.mnemosyne`；已有 `plugins:` 块 → 在其下插入 `mnemosyne:` 而不破坏同级其他插件；已有 `memory.provider: ''` → 被替换为 `mnemosyne`；其余键逐字保留。
   - **幂等**：连跑两次，config 不重复追加、结果稳定。
   - **备份**：生成 `config.yaml.mnemosyne-bak-*`。
   - `--dry-run` 不落盘只打印；`--no-config` 跳过 config 改动。

> 注：测试不依赖真实 `~/.hermes`，全部用临时目录与可控 bridge（必要时 monkeypatch `_run` 或指向临时 store 的真实子进程）。config 写入用准备好的 fixture 文本断言结果。

## 7. 交付物清单

- `mnemosyne/integrations/__init__.py`
- `mnemosyne/integrations/hermes/__init__.py`（provider + `register(ctx)`）
- `mnemosyne/integrations/hermes/plugin.yaml`
- `mnemosyne/integrations/hermes/README.md`
- `mnemosyne/cli.py`：新增 `install-hermes` 子命令
- `tests/test_hermes_provider.py`
- `README.md`：补充 Hermes 接入章节
- 安装到 `~/.hermes/plugins/mnemosyne/` 并在 `~/.hermes/config.yaml` 启用（部署步骤，非 git 交付物）

## 8. 非目标（YAGNI）

- 不做每轮对话的自动记忆抽取（保持共享 store 干净）。
- 不替换/改动 Hermes 内置记忆系统，二者并存。
- 不做进程内 import / 不给 Hermes venv 加依赖。
- 不动 Mnemosyne 检索/store 内核逻辑——provider 纯粹是 CLI 之上的薄适配层。
