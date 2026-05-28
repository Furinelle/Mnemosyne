# Mnemosyne

> 希腊记忆女神，缪斯九姐妹之母。

面向 LLM Agent 的**可迁移通用记忆系统**。专为 Claude Code、Codex 等能调用 Shell 和 Python 的 Agent 设计，让记忆跨对话持久保存、跨 Agent 共享。

- **文件即接口**：记忆存为 Markdown + YAML frontmatter，任何工具可读可写
- **分层存储**：全局偏好 + 项目知识，互不污染
- **持久检索索引**：SQLite FTS5 + BM25 fallback，无需云端向量库
- **自动生命周期**：强度衰减、热度晋升、冷数据归档
- **Claude Code 原生集成**：4 个 hooks 自动注入 / 触发写入 / 后台维护
- **跨 Agent 协作**：Codex 等外部 Agent 通过 `codex-prep` / `codex-ingest` 双向同步

依赖：Python ≥ 3.11，`portalocker ≥ 2.8`（自动安装，用于并发写锁）。

---

## 5 分钟上手

### 安装

```bash
git clone https://github.com/Furinelle/Mnemosyne
pip install -e ./Mnemosyne
```

安装后 `python -m mnemosyne` 在任意目录均可使用。

### 初始化项目

```bash
cd your-project
python -m mnemosyne init
```

会在当前目录创建 `.mnemosyne/` 并向项目根写入 `AGENTS.md` 模板（供 Codex 等外部 Agent 读取）：

```
.mnemosyne/
  core.md       ← 每次任务必载的核心约束（手动维护）
  config.toml   ← 阈值配置
  working/      ← 活跃记忆（自动写入）
  archive/      ← 冷归档（自动维护）
```

### 基础用法：手动 CLI

```bash
# 写入一条踩坑记录
python -m mnemosyne write --type pitfall --importance 80 \
  --title "JWT 不能存 localStorage" \
  --content "存 localStorage 会被 XSS 直接读取，改 httpOnly cookie。"

# 搜索
python -m mnemosyne search "JWT 认证" --limit 3

# 重建持久搜索索引
python -m mnemosyne reindex

# 维护（衰减、归档）
python -m mnemosyne maintain

# 诊断本机配置
python -m mnemosyne doctor
```

### 进阶 1：启用 Claude Code 自动注入

把仓库自带的 hooks 配置合并到 Claude Code 的 `~/.claude/settings.json` 或 `.claude/settings.json`：

```bash
cat path/to/Mnemosyne/templates/settings.json
```

配好后 Claude Code 将在以下时机自动调用 Mnemosyne，**完全无需手动 read/search/write**：

| 时机 | Hook | 行为 |
|---|---|---|
| 会话开始 | `SessionStart` | 注入 global + project core memory；24h 节流跑后台 maintain；**自动 init**：进入任意 git 项目首次会话时自动创建 `.mnemosyne/` |
| 用户提交 prompt | `UserPromptSubmit` | 用 prompt 关键词搜 top-3 相关记忆并注入 |
| Edit / Write 工具前 | `PreToolUse` | 用目标文件名搜 top-2 相关记忆并注入 |
| 会话结束 | `Stop` | dry-run maintain 提示 Core 晋升候选 |

Claude 何时**主动写入**记忆由 `templates/CLAUDE.md` 模板定义的触发条件控制（踩坑 / 架构决策 / 用户偏好 / 代码库知识 / Codex 交接）。把这份模板加入你的 `~/.claude/CLAUDE.md` 即可。

### 进阶 2：全局自动启用

配好 SessionStart hook 后，**进入任意 git 项目第一次开 Claude Code 时，`.mnemosyne/` 会自动创建**——无需手动跑 `mnemosyne init`。

判定逻辑：从 cwd 向上找到 `.git` 目录或文件即视为项目根，且会避开"家目录本身是 git repo"的极端情况（不会与全局 store 冲突）。auto-init 只创建 store 骨架，**不会**在项目根写 `AGENTS.md`——Codex 用全局 `~/.codex/AGENTS.md` 就够了，避免污染你的 repo。

退出方式（按需选一种）：

```bash
# 单个项目永久退出：在项目根放个空标记文件
touch your-project/.mnemosyne-disable

# 全局退出（环境变量）：
export MNEMOSYNE_AUTO_INIT=0
```

**配合全局 gitignore**：在 `~/.config/git/ignore`（或你的 `core.excludesfile`）里加一行 `.mnemosyne/`，从此所有 repo 都自动忽略生成的记忆文件，不会污染版本控制：

```bash
echo '.mnemosyne/' >> ~/.config/git/ignore
echo '.mnemosyne-disable' >> ~/.config/git/ignore
```

### 进阶 3：与 Codex 双向协作

向 Codex 派任务前，生成含 core memory + 相关历史的 prompt 前缀：

```bash
python -m mnemosyne codex-prep "调试 JWT 认证失败"
```

输出可直接拼到 Codex 的输入。`AGENTS.md`（init 自动写出）会告诉 Codex 在回复末尾追加 `**新发现:**` 块。

Codex 完成后，把它的输出喂回 ingest 命令：

```bash
echo "$CODEX_OUTPUT" | python -m mnemosyne codex-ingest --source codex --commit
```

默认是 dry-run（只 preview 不写）；加 `--commit` 才真写入。

---

## 核心概念

### 记忆分层

| 层 | 位置 | 内容 | 载入方式 |
|---|---|---|---|
| **Core** | `core.md` | 永远不能违反的约束、项目身份 | SessionStart hook 自动注入 |
| **Working** | `working/` | 活跃的决策、踩坑、交接记录 | UserPromptSubmit / PreToolUse hook 按需检索 |
| **Archive** | `archive/YYYY-MM/` | 强度衰减后的冷记忆 | 显式 `search --archive` |

### 记忆类型

| 类型 | 用途 | 典型内容 |
|---|---|---|
| `arch_decision` | 架构决策 | 为什么选 A 不选 B，被否决方案的劣势 |
| `pitfall` | 踩坑记录 | 现象、根因、修复方式、如何避免 |
| `codebase` | 代码库知识 | 模块职责、关键入口、依赖关系 |
| `preference` | 用户偏好 | 用户的编码风格、工具偏好、反馈模式 |
| `handoff` | Agent 交接 | Codex 任务的新发现、遗留问题 |

### 强度（Strength）机制

每条记忆有 `strength` 字段（0–100），模拟遗忘曲线：

```
写入时：strength = --importance 指定的值
搜索命中（Working）：strength += 5
搜索命中（Archive，重新激活）：strength += 20
每次 maintain：strength -= 1

strength < 30  → 移入 Archive
strength < 5   → 标记 deprecated
strength ≥ 80 且 access_count ≥ 3 → 打印为 Core 晋升候选
```

阈值可在 `.mnemosyne/config.toml` 调整。

---

## 命令参考

| 命令 | 用途 |
|---|---|
| `init` | 创建 `.mnemosyne/` 并写出 `AGENTS.md` |
| `read [--scope all\|global\|project]` | 输出 core memory 用于 prompt 注入 |
| `write --type T --importance N ...` | 写入一条记忆 |
| `search QUERY [--type T] [--scope ...] [--archive] [--format json]` | BM25 搜索 |
| `show ID` | 查看完整记忆（含 frontmatter） |
| `link ID1 ID2 --rel REL` | 建立双向链接 |
| `maintain [--scope ...] [--dry-run]` | 衰减、归档、列出 Core 候选 |
| `reindex [--scope ...] [--no-archive]` | 重建 SQLite FTS5 持久搜索索引 |
| `doctor [--scope ...]` | 检查依赖、模板、store、FTS5、索引状态 |
| `codex-prep TASK [--limit 5]` | 生成 Codex handoff prompt 前缀 |
| `codex-ingest [--source NAME] [--commit]` | 从 stdin 解析 `**新发现:**` 块（默认 dry-run） |

### `write` 完整参数

| 参数 | 说明 | 默认 |
|---|---|---|
| `--type` | 记忆类型（必填） | — |
| `--importance` | 初始强度 0–100（必填） | — |
| `--title` | 标题 | 自动从首行提取 |
| `--tags` | 逗号分隔标签 | 空 |
| `--content` | 内容（或通过 stdin） | 空 |
| `--expires` | 失效条件描述 | 空 |
| `--scope` | `global` / `project` | `project` |
| `--source` | 来源 Agent 名 | `agent` |
| `--force` | 跳过去重确认 | 否 |

**去重**：未加 `--force` 且为交互式时，写入前自动 BM25 搜索，相似度高会询问是否合并。Hook 路径自动 `--force`。

### `maintain`

```bash
python -m mnemosyne maintain --dry-run   # 预览
python -m mnemosyne maintain --scope all # 全局 + 项目
```

执行：所有活跃记忆 `strength -= 1`；< 30 移归档；< 5 标 deprecated；≥ 80 且 access ≥ 3 打印晋升候选。**SessionStart hook 已配 24h 节流后台触发**，无需手动跑。

---

## Hooks 详解

Mnemosyne 提供 5 个 hook 模块，挂载到 Claude Code 后实现读写全自动化。模板见 [`templates/settings.json`](templates/settings.json)。

| 模块 | 触发 event | 行为 | 超时 |
|---|---|---|---|
| `mnemosyne.hooks.session_start` | `SessionStart` | 注入 core memory；后台跑 maintain（24h 节流）；在 git 项目自动 init `.mnemosyne/` | 10s |
| `mnemosyne.hooks.user_prompt_submit` | `UserPromptSubmit` | 用 prompt 关键词搜 top-3 注入 | 5s |
| `mnemosyne.hooks.pre_tool_use` | `PreToolUse` (Edit\|Write) | 用文件名搜 top-2 注入 | 5s |
| `mnemosyne.hooks.stop` | `Stop` | dry-run maintain，提示 Core 晋升候选 | 5s |

所有 hook 都包了 `hook_safe()` 装饰器：**任何异常 stderr trace 后退 0，不阻塞 Claude**。

写入触发由 Claude 主动判断，依据 [`templates/CLAUDE.md`](templates/CLAUDE.md) 中的"任务结束前主动写入触发条件"清单。

---

## Codex 双向通道

```
Claude Code ──┐
              ├─► codex-prep ──► Codex (prompt 前缀含 core + 相关记忆 + AGENTS.md 指引)
              │                       │
              │                       ▼
              │                  Codex 回复末尾追加 "**新发现:**" 块
              │                       │
              └◄─── codex-ingest ◄────┘
                  (--commit 写入项目记忆,source=codex)
```

- **`codex-prep TASK`**：拼接 core memory + 任务相关 top-K 记忆 + Mnemosyne CLI 指引 + `**新发现:**` 模板
- **`codex-ingest`**：从 stdin 读 Codex 全文，正则匹配 `**新发现:**` / `**Findings:**` 块。非法 type / 空 content / 坏 importance 的 finding 会被 stderr warn 后丢弃，不会让 ingest 失败
- 鲁棒性：parse_findings 入口 lstrip BOM，兼容 PowerShell pipe 工件

---

## 存储结构

```
~/.mnemosyne/              # 全局（用户偏好、跨项目习惯）
  core.md
  working/
  archive/YYYY-MM/

your-project/.mnemosyne/   # 项目
  core.md
  config.toml
  working/
  archive/YYYY-MM/
  .lock                    # 目录级排他锁（maintain 使用）
  .last_maintain           # 24h 节流标记
```

全局存储路径可通过环境变量覆盖：

```bash
export MNEMOSYNE_HOME=/path/to/custom/store
```

**并发安全**：所有写入走 `tmp + os.replace` 原子重命名 + portalocker 排他锁。多 Agent 同时跑不冲突。`cmd_search` 的访问统计回写用 timeout=0 try-lock，拿不到锁时跳过统计但搜索结果照常返回。

---

## 记忆文件格式

每条记忆是一个带 YAML frontmatter 的 Markdown 文件：

```markdown
---
id: pitfall-2026-05-18-0c0a59
type: pitfall
source: codex
strength: 80
created: 2026-05-18
last_accessed: 2026-05-18
access_count: 2
tags: [auth, xss, security]
links:
  - id: arch_decision-2026-05-18-9a8341
    rel: caused_by
canonical_summary: "JWT XSS 漏洞: 存 localStorage 导致 token 被 XSS 读取"
injection_summary: "JWT XSS 漏洞: 存 localStorage 导致 token 被 XSS 读取，改 httpOnly cookie + CSRF。"
status: active
expires: 认证方案重构时失效
---

## JWT XSS 漏洞

存 localStorage 导致 token 被 XSS 读取，改 httpOnly cookie + CSRF。
```

`source` 字段用于追溯写入方（`claude-code` / `codex` / `agent` / `user` 等），可用 `search --format json` 过滤。

---

## 配置

`.mnemosyne/config.toml`：

```toml
[thresholds]
decay_per_run = 1       # 每次 maintain 衰减量
bonus_access = 5        # 搜索命中时强度增量
bonus_recall = 20       # 从 Archive 召回时强度增量
core_strength = 80      # Core 晋升候选的强度门槛
core_access_count = 3   # Core 晋升候选的访问次数门槛
archive_strength = 30   # 移入 Archive 的强度阈值
deprecated_strength = 5 # 标记 deprecated 的强度阈值

[memory]
types = ['arch_decision', 'pitfall', 'codebase', 'preference', 'handoff']

[injection]
max_tokens = 2000       # hook 注入记忆的近似 token 上限

[search]
index_enabled = true    # 优先使用 SQLite FTS5 持久索引，失败时回退内存 BM25
```

### 搜索索引

Mnemosyne 会在每个 store 下维护 `index.sqlite`。写入记忆时会增量更新索引；`search`
在索引不存在时会自动创建。需要全量修复时运行：

```bash
python -m mnemosyne reindex --scope all
```

搜索 JSON 输出会包含 `why_matched`，用于解释命中的文本片段。

---

## License

MIT
