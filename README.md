# Mnemosyne

> 希腊记忆女神，缪斯九姐妹之母。

面向 LLM Agent 的**可迁移通用记忆系统**。专为 Claude Code、Codex 等能调用 Shell 和 Python 的 Agent 设计，让记忆跨对话持久保存、跨 Agent 共享。

- **零外部依赖**：纯 Python 标准库，`>= 3.10`
- **文件即接口**：记忆存为 Markdown + YAML frontmatter，任何工具可读可写
- **分层存储**：全局偏好 + 项目知识，互不污染
- **BM25 检索**：关键词搜索，无需向量数据库
- **自动生命周期**：强度衰减、热度晋升、冷数据归档

---

## 快速上手

### 安装

```bash
git clone https://github.com/Furinelle/Mnemosyne
pip install -e ./Mnemosyne
```

安装后 `python -m mnemosyne` 在任意目录均可使用。

### 在项目里初始化

```bash
cd your-project
python -m mnemosyne init
```

会在当前目录创建 `.mnemosyne/`，包含：

```
.mnemosyne/
  core.md       ← 每次任务必载的核心约束（手动维护）
  config.toml   ← 阈值配置
  working/      ← 活跃记忆（自动写入）
  archive/      ← 冷归档（自动维护）
```

---

## 核心概念

### 记忆分层

| 层 | 位置 | 内容 | 载入方式 |
|---|---|---|---|
| **Core** | `core.md` | 永远不能违反的约束、项目身份 | 每次必载 |
| **Working** | `working/` | 活跃的决策、踩坑、交接记录 | 按需检索 |
| **Archive** | `archive/YYYY-MM/` | 强度衰减后的冷记忆 | 显式搜索 |

### 记忆类型

| 类型 | 用途 | 典型内容 |
|---|---|---|
| `arch_decision` | 架构决策 | 为什么选择这种方案，被否定的方案是什么 |
| `pitfall` | 踩坑记录 | 现象、根因、修复方式、如何避免 |
| `codebase` | 代码库知识 | 模块职责、关键入口、依赖关系 |
| `preference` | 用户偏好 | 用户的编码风格、工具偏好、反馈模式 |
| `handoff` | Agent 交接 | Codex 任务的新发现、遗留问题 |

### 强度（Strength）机制

每条记忆有 `strength` 字段（0–100），模拟遗忘曲线：

```
写入时：strength = --importance 指定的值
被搜索命中：strength += 5（Working）/ += 20（Archive，重新激活）
每次 maintain：strength -= 1

strength < 30 → 移入 Archive
strength < 5  → 标记 deprecated，不再参与检索
strength >= 80 且访问次数 >= 3 → 打印为 Core 晋升候选
```

---

## 命令详解

### `init` — 初始化项目记忆

```bash
python -m mnemosyne init
```

在当前目录创建 `.mnemosyne/` 结构。**第一步必做。**

---

### `read` — 读取 Core Memory

```bash
python -m mnemosyne read                # 只读项目 core.md
python -m mnemosyne read --scope all    # 全局 + 项目
python -m mnemosyne read --scope global # 只读全局
```

输出内容适合直接注入 Agent 的 prompt。**在每次任务开始前调用。**

---

### `write` — 写入记忆

```bash
python -m mnemosyne write \
  --type pitfall \
  --importance 80 \
  --title "JWT 不能存 localStorage" \
  --tags "auth,security" \
  --content "JWT 存入 localStorage 会导致 XSS 攻击可直接读取。必须用 httpOnly cookie。" \
  --expires "认证方案重构时失效"
```

| 参数 | 说明 | 默认值 |
|---|---|---|
| `--type` | 记忆类型（必填） | — |
| `--importance` | 初始强度 0–100（必填） | — |
| `--title` | 标题 | 自动从内容提取 |
| `--tags` | 逗号分隔标签 | 空 |
| `--content` | 内容（或通过 stdin 传入） | 空 |
| `--expires` | 失效条件描述 | 空 |
| `--scope` | `global` 或 `project` | `project` |
| `--source` | 来源 Agent 名称 | `agent` |
| `--force` | 跳过去重确认提示 | 否 |

**去重机制**：写入前自动 BM25 搜索，相似度高时询问是否合并，避免重复条目积累。

---

### `search` — 搜索记忆

```bash
python -m mnemosyne search "auth 认证"
python -m mnemosyne search "性能问题" --type pitfall --limit 3
python -m mnemosyne search "模块依赖" --scope all --archive
python -m mnemosyne search "配置" --format json   # 供 Agent 机器解析
```

| 参数 | 说明 |
|---|---|
| `--scope` | `global` / `project` / `all` |
| `--type` | 按类型过滤 |
| `--limit` | 返回条数（默认 5） |
| `--format` | `text`（默认）或 `json` |
| `--archive` | 同时搜索归档记忆 |

搜索命中会自动更新 `access_count` 和 `strength`（+5）。

---

### `maintain` — 维护记忆生命周期

```bash
python -m mnemosyne maintain             # 正式执行
python -m mnemosyne maintain --dry-run   # 预览，不实际修改
python -m mnemosyne maintain --scope project
```

执行内容：
1. 所有活跃记忆 `strength -= 1`
2. `strength < 30` → 移入 `archive/YYYY-MM/`
3. `strength < 5` → 标记 `deprecated`
4. `strength >= 80` 且 `access_count >= 3` → 打印为 Core 晋升候选（需手动移入 `core.md`）

**建议每周运行一次。**

---

### `show` — 查看完整记忆

```bash
python -m mnemosyne show pitfall-2026-05-18-0c0a59
```

输出记忆的完整 Markdown 内容，包含所有 frontmatter 字段。

---

### `link` — 关联两条记忆

```bash
python -m mnemosyne link arch_decision-xxx pitfall-yyy --rel caused_by
```

在两条记忆之间建立双向链接，支持关联语义（`caused_by` / `related_to` / `supersedes` 等）。链接在 `show` 和 `search --format json` 中可见。

---

## 与 Agent 集成

### Claude Code

在项目 `.claude/CLAUDE.md` 中添加以下规则，Claude Code 会自动读写记忆：

```markdown
## 记忆系统

**任务开始前：**
运行 `python -m mnemosyne read --scope all`。
运行 `python -m mnemosyne search "<任务关键词>" --limit 3`。

**任务完成后，遇到以下情况自动写入：**
- 做了架构决策 → `--type arch_decision`
- 踩坑或修复 bug → `--type pitfall`
- 用户纠正了行为 → `--type preference`
- Codex 返回新发现 → `--type handoff`

写入命令：
python -m mnemosyne write --type <类型> --importance <50-90> \
  --source claude-code --force --title "<标题>" --content "<内容>"
```

### Codex（push + pull 双模式）

Claude Code 委派任务时，在 prompt 末尾附加：

```
记忆 CLI 可用：python -m mnemosyne search "<关键词>" --format json
如执行中需要更多上下文，主动搜索。
完成后在回复末尾附：
**新发现：** <值得记录的内容，无则省略>
```

- **push**：Claude Code 预先检索相关记忆注入 prompt
- **pull**：Codex 执行中途主动调用 `search` 获取更多上下文
- **write back**：Claude Code 将 Codex 的 findings 写入 `--type handoff`

---

## 存储结构

```
~/.mnemosyne/              # 全局（用户偏好、跨项目习惯）
  core.md
  working/
  archive/YYYY-MM/

your-project/.mnemosyne/   # 项目（架构决策、踩坑、交接）
  core.md
  config.toml
  working/
  archive/YYYY-MM/
```

全局存储路径可通过环境变量覆盖：

```bash
export MNEMOSYNE_HOME=/path/to/custom/store
```

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
canonical_summary: "JWT XSS 漏洞: JWT 存入 localStorage 导致 XSS 可读取 token"
injection_summary: "JWT XSS 漏洞: JWT 存入 localStorage 导致 XSS 可读取 token，改用 httpOnly cookie + CSRF 双保险。"
status: active
expires: 认证方案重构时失效
---

## JWT XSS 漏洞

JWT 存入 localStorage 导致 XSS 可读取 token，改用 httpOnly cookie + CSRF 双保险。
```

---

## 配置

`.mnemosyne/config.toml`（Python 3.11+ 才会读取，3.10 静默跳过使用默认值）：

```toml
[thresholds]
decay_per_run = 1       # 每次 maintain 衰减量
bonus_access = 5        # 被搜索命中时强度增量
bonus_recall = 20       # 从 Archive 被召回时强度增量
core_strength = 80      # Core 晋升候选的强度门槛
core_access_count = 3   # Core 晋升候选的访问次数门槛
archive_strength = 30   # 移入 Archive 的强度阈值
deprecated_strength = 5 # 标记 deprecated 的强度阈值

[memory]
types = ['arch_decision', 'pitfall', 'codebase', 'preference', 'handoff']
```

---

## License

MIT
