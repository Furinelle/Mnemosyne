# Mnemosyne

> 希腊记忆女神，缪斯九姐妹之母。

Mnemosyne 是一个面向本地 LLM Agent 的共享记忆层。它把长期记忆保存为
Markdown 文件，让 Claude Code、Codex、Hermes 等能调用 Shell/Python 的 Agent
可以在不同会话、不同工具之间读取同一份项目知识、用户偏好和交接记录。

它最适合解决这类问题：

- Claude Code、Codex 桌面版和 Hermes 之间缺少同一套长期记忆。
- Agent 每次开新会话都要重新解释项目约束、踩坑和偏好。
- 项目记忆需要能被人直接查看、审计、编辑，而不是锁在某个产品里。
- 希望全局偏好和项目知识分开保存，避免互相污染。

## 功能概览

| 能力 | 说明 |
|---|---|
| 文件即接口 | 每条记忆都是 Markdown + YAML frontmatter，任何编辑器都能读写。 |
| 全局 + 项目分层 | `~/.mnemosyne/` 保存跨项目偏好，项目内 `.mnemosyne/` 保存项目知识。 |
| 持久搜索索引 | 优先使用 SQLite FTS5 的 `index.sqlite`，不可用时回退到内存 BM25。 |
| 混合检索 | CJK bigram、可选向量 lane、RRF 融合、关系扩展和可选 reranker。 |
| MCP 服务 | 通过 stdio 或可选 SSE 向 Cursor、Cline、Continue、Windsurf 等客户端暴露记忆工具。 |
| 关系图谱 | typed links、关系权重扩展，以及 Mermaid、ASCII、JSON 三种 graph 输出。 |
| 生命周期管理 | 记忆会按强度衰减、归档、召回，过期（`expires`）记忆自动归档，并提示可晋升到 core memory 的候选。 |
| Claude Code hooks | 支持 SessionStart、UserPromptSubmit、PreToolUse、Stop 四个自动注入点。 |
| Codex 交接 | 提供 `codex-prep` 和 `codex-ingest`，也可通过 `AGENTS.md` 让 Codex 直接读写。 |
| Hermes 原生集成 | `install-hermes` 一键安装原生 MemoryProvider 插件，重启后自动注入与检索。 |
| 跨 agent 自动记忆形成 | 可选的 `distill` 引擎（heuristic/LLM/host）从会话 transcript 中抽取记忆，默认关闭，写入前会去重/supersede。 |
| LongMemEval 基准 | `eval convert/fetch longmemeval` 和 `eval run --longmemeval` 提供 per-instance recall/MRR 与按问题类型拆分。 |

基础依赖：Python 3.11+，`portalocker>=2.8`。

## v0.2 三轨升级

### 检索质量

v0.2 把连续 CJK 文本拆成 bigram，因此查询“认证”可以召回“调试认证失败”。FTS5
索引使用 trigram tokenizer；遇到 SQLite trigram 无法覆盖的两字 CJK 查询时，会自动
走受控的 substring 回退。英文和数字 token 仍保持原来的 BM25 行为。

向量检索和 cross-encoder rerank 都是可选能力，默认关闭。启用 embedding 后，
Mnemosyne 会把 float16 向量保存到 SQLite 元数据表，并用 RRF 融合 BM25 与 vector
候选；`mnemosyne eval run` 可以在固定 50 条语料上输出可复现的 recall、MRR 和延迟。

### MCP 服务化

`mnemosyne mcp serve` 把搜索、写入、读取 core、查看、链接、图谱、维护和
`codex-prep` 暴露为 MCP tools。默认传输是 stdio，适合本地编辑器和 Agent；
`mnemosyne mcp serve --sse` 可用于调试或远程接入。

MCP SDK 是可选 extra。没有安装时，原有 CLI 和 hooks 完全不受影响；只有启动 MCP
server 时会提示安装 `mnemosyne[mcp]`。仓库内的 `templates/mcp_clients/` 提供
Cursor、Cline、Continue 和 Windsurf 的配置片段。

### 关系图谱

`link` 现在支持 `caused_by`、`refines`、`supersedes`、`contradicts`、`related`
五种预定义关系。非对称关系会自动写入对应的反向语义，例如
`A --rel supersedes B` 会让 B 指向 A 的关系变成 `superseded_by`，并降低被取代记忆 B 的强度。

搜索会按关系类型对链接记忆加权扩展。`graph ID` 可以从任意记忆做 BFS，并输出
Mermaid、ASCII 或 JSON；自定义关系默认拒绝，需要显式加 `--allow-custom`。

## v0.3 自动记忆形成与基准

### 跨 agent 自动记忆形成（distill）

`distill` 可以从会话 transcript 里抽取值得长期保留的记忆，默认关闭（opt-in）。
启用方式是把项目 `config.toml` 里的 `[distill].enabled` 设为 `true`，并选择引擎：
`heuristic`（默认，纯 stdlib 启发式，无额外依赖）、`llm`（调用 `[distill.llm]`
配置的 API）或 `host`（解析 agent 输出里的 `**新发现:**` 块）。写入前会先做
去重和 supersede 判定，避免和已有记忆冲突或重复。三个触发点：Claude Code 的
Stop hook、Codex 的 `codex-ingest --commit`、Hermes provider 的
`on_session_end`。也可以手动调用：

```bash
python3 -m mnemosyne distill --transcript /path/to/transcript.jsonl --commit
```

不加 `--commit` 时是 dry-run，只打印将要写入的候选记忆。

### LongMemEval 基准

```bash
python3 -m mnemosyne eval convert longmemeval --raw raw.json --out ./longmemeval
python3 -m mnemosyne eval run --longmemeval --by-type --pipeline full
```

`convert` 把官方 LongMemEval 数据转换成 Mnemosyne 的 seed memories + corpus；
`fetch` 尝试直接下载（官方地址目前是占位符，下载失败会提示手动获取）。
`eval run --longmemeval` 按 instance 隔离评分，输出 recall@1/5/10 和 MRR，
`--by-type` 按问题类型拆分，`--pipeline full` 会经过真实的 FTS5 + RRF
fusion 检索栈而不是 toy BM25。

## 快速开始

### 1. 安装

```bash
git clone https://github.com/Furinelle/Mnemosyne
cd Mnemosyne
python3 -m pip install -e .
python3 -m mnemosyne doctor
```

按需安装可选能力：

```bash
python3 -m pip install -e ".[vector]"   # numpy + onnxruntime
python3 -m pip install -e ".[rerank]"   # cross-encoder runtime
python3 -m pip install -e ".[mcp]"      # MCP server SDK
```

安装后推荐继续使用 `python3 -m mnemosyne ...`，这样不依赖 shell 是否能找到
`mnemosyne` 可执行脚本。若你的环境已经能直接运行 `mnemosyne`，两种写法等价。

### 2. 初始化一个项目

进入任意 Git 项目根目录：

```bash
python3 -m mnemosyne init
```

它会创建：

```text
.mnemosyne/
  core.md
  config.toml
  working/
  archive/
```

`init` 还会在项目根目录写入 `AGENTS.md` 模板。如果项目已经有 `AGENTS.md`，它不会覆盖。
`MEMORY.md` 和 `index.sqlite` 会在后续写入、搜索或重建索引时按需生成。

建议把本地记忆目录加入全局 gitignore：

```bash
mkdir -p ~/.config/git
printf ".mnemosyne/\n.mnemosyne-disable\n" >> ~/.config/git/ignore
```

如果你希望团队共享项目记忆，也可以选择把 `.mnemosyne/` 纳入版本控制。

### 3. 写入和搜索记忆

```bash
python3 -m mnemosyne write \
  --type pitfall \
  --importance 80 \
  --source user \
  --tags "auth,security" \
  --title "JWT 不要存 localStorage" \
  --content "localStorage 中的 token 会被 XSS 直接读取，改用 httpOnly cookie。"

python3 -m mnemosyne search "JWT 认证" --limit 3
python3 -m mnemosyne show pitfall-2026-05-28-xxxxxx
```

常用记忆类型：

| 类型 | 适合记录 |
|---|---|
| `preference` | 用户长期偏好。通常用 `--scope global`。 |
| `codebase` | 项目结构、关键入口、模块职责。 |
| `pitfall` | 非显然的 bug、根因、修复和避免方式。 |
| `arch_decision` | 架构选择，以及为什么没有选其它方案。 |
| `handoff` | 一个 Agent 给下一个 Agent 的交接摘要。 |

## 让 Claude Code、Codex 和 Hermes 共享记忆

Mnemosyne 的共享方式很朴素：Claude Code 和 Codex 都读写同一套
`~/.mnemosyne/` 与项目 `.mnemosyne/` 文件。

### Claude Code Desktop

把 hooks 模板合并到 Claude Code 的全局或项目 settings：

```bash
cat templates/settings.json
```

常用目标文件：

- 全局：`~/.claude/settings.json`
- 项目：`.claude/settings.json`

模板中默认命令是 `python3 -m mnemosyne...`。如果你的机器需要固定解释器，
可以把模板里的 `python3` 改成 `which python3` 输出的绝对路径。

启用后 Claude Code 会自动做这些事：

| 时机 | Hook | 行为 |
|---|---|---|
| 会话开始 | `SessionStart` | 注入 global + project core memory，后台维护记忆，并按需自动创建 `.mnemosyne/`。 |
| 用户提交 prompt | `UserPromptSubmit` | 根据 prompt 搜索相关记忆并注入上下文。 |
| Edit / Write 前 | `PreToolUse` | 根据目标文件名搜索相关记忆并注入。 |
| 会话结束 | `Stop` | dry-run 维护，提示可晋升到 core memory 的候选；若 `[distill].enabled = true`，自动从 transcript 蒸馏并写入记忆。 |

再把 `templates/CLAUDE.md` 的规则加入你的 `~/.claude/CLAUDE.md`，Claude Code 就会在
遇到踩坑、架构决策、用户偏好、代码库知识或交接信息时主动写入记忆。

### Codex Desktop

Codex 没有 Claude Code hooks，但可以通过 `AGENTS.md` 约定在任务开始和结束时调用
Mnemosyne CLI。

推荐全局配置：

```bash
mkdir -p ~/.codex
[ -f ~/.codex/AGENTS.md ] || cp templates/AGENTS.md ~/.codex/AGENTS.md
```

如果 `~/.codex/AGENTS.md` 已经存在，把 `templates/AGENTS.md` 里的 Mnemosyne 规则合并进去即可。
如果你还想给某个项目单独配置，运行 `python3 -m mnemosyne init` 后会生成项目级 `AGENTS.md`。

Codex 也可以手动走交接命令：

```bash
python3 -m mnemosyne codex-prep "调试 JWT 认证失败"
```

把输出放到 Codex prompt 前缀里。Codex 完成后，如果回复末尾包含 `**新发现:**`
块，可以写回记忆：

```bash
echo "$CODEX_OUTPUT" | python3 -m mnemosyne codex-ingest --source codex --commit
```

不加 `--commit` 时只是 dry-run 预览，不会真正写入。

如果项目或全局 `AGENTS.md` 末尾输出了 `**新发现:**` 块，`codex-ingest --commit`
就是 Codex 侧的自动记忆形成入口；它会解析块内容并写入共享 store。

### 全局自动启用

配置 Claude Code 的 `SessionStart` hook 后，进入任意 Git 项目时，Mnemosyne 会自动创建
`.mnemosyne/` 骨架。auto-init 只创建记忆目录，不会写项目 `AGENTS.md`，避免污染仓库。

退出方式：

```bash
# 单个项目禁用
touch .mnemosyne-disable

# 全局禁用
export MNEMOSYNE_AUTO_INIT=0
```

### MCP 客户端

先安装 MCP extra，再选择客户端模板：

```bash
python3 -m pip install -e ".[mcp]"
cat templates/mcp_clients/cursor.json
```

模板默认启动 `mnemosyne mcp serve`。把对应 JSON 片段合并到 Cursor、Cline、
Continue 或 Windsurf 的 MCP 配置后，客户端即可发现 8 个 `mnemosyne_*` tools。

### Hermes

让 Hermes 桌面/网关 Agent 也共享同一套 Mnemosyne 记忆：

```bash
python3 -m mnemosyne install-hermes
```

这条命令会做三件事：

1. 把原生 MemoryProvider 插件写入 `~/.hermes/plugins/mnemosyne/`。
2. 自动备份原有 `config.yaml`，然后在其中设置 `memory.provider: mnemosyne`。
3. 打印安装摘要，提示下一步。

**重启 Hermes** 后，集成立即生效。启用后 Hermes 会自动做这些事：

| 时机 | 行为 |
|---|---|
| 会话开始 | 读取 global + project `core.md`，注入上下文。 |
| 每轮对话前 | 按当前 prompt 搜索相关 working 记忆，追加到上下文。 |
| 工具调用 / 写操作 | 写入记忆时自动带 `--source hermes`，写入共享 store。 |
| 会话结束 | 若 `[distill].enabled = true`，通过 `on_session_end` 自动调用 `distill --stdin --commit --source hermes`。 |

Hermes 侧暴露的记忆工具与 Claude Code 相同：`search`、`write`、`show`、`link`、`graph`，
均操作同一份 `~/.mnemosyne/` 和项目 `.mnemosyne/` 文件。

**预览不写入**：

```bash
python3 -m mnemosyne install-hermes --dry-run
```

**卸载**：删除 `~/.hermes/plugins/mnemosyne/` 并将 `config.yaml` 回滚到备份即可。

详见 `mnemosyne/integrations/hermes/README.md`。

## 记忆模型

### Core、Working、Archive

| 层级 | 位置 | 用途 | 载入方式 |
|---|---|---|---|
| Core | `core.md` | 永远需要知道的约束、身份、项目原则。 | `read` 和 SessionStart 自动注入。 |
| Working | `working/` | 活跃记忆，如踩坑、决策、偏好、交接。 | `search` 或 hooks 按需检索。 |
| Archive | `archive/YYYY-MM/` | 强度衰减后的冷记忆。 | `search --archive` 显式检索。 |

### Strength

每条记忆都有 `strength`，用于控制遗忘和召回：

```text
写入时：strength = --importance
搜索命中 working：strength += bonus_access，默认 +5
搜索命中 archive：strength += bonus_recall，默认 +20
maintain 时：strength -= decay_per_run，默认 -1

strength < archive_strength，默认 30：移入 archive
strength < deprecated_strength，默认 5：标记 deprecated
strength >= core_strength 且 access_count >= core_access_count：提示晋升到 core.md
```

## 常用命令

| 命令 | 用途 |
|---|---|
| `init` | 在当前目录创建项目 `.mnemosyne/`，并写入 `AGENTS.md` 模板。 |
| `read --scope all` | 输出 global + project core memory，适合 prompt 注入。 |
| `write --type T --importance N ...` | 写入一条记忆。 |
| `search QUERY --format json` | 搜索记忆；优先使用 SQLite FTS5，回退到内存 BM25。 |
| `show ID` | 查看一条完整记忆，包括 frontmatter。 |
| `link ID1 ID2 --rel REL` | 用 typed relation 链接两条记忆；自定义关系需 `--allow-custom`。 |
| `graph ID --format mermaid` | BFS 展开关系图，支持 `mermaid`、`ascii`、`json`。 |
| `maintain --dry-run` | 预览衰减、归档和 core 晋升候选。 |
| `maintain --scope all` | 维护全局和项目记忆。 |
| `reindex --scope all` | 全量重建搜索索引。 |
| `embed-backfill --scope all` | 为已有记忆计算或刷新 embedding。 |
| `eval run --corpus FILE` | 输出固定语料的 recall、MRR 和延迟基线。 |
| `eval convert longmemeval --raw FILE --out DIR` | 把 LongMemEval 原始数据转换成 seed memories + corpus。 |
| `eval fetch longmemeval --variant {s,m}` | 下载 LongMemEval 数据集（官方地址未确认前会提示手动下载）。 |
| `eval run --longmemeval [--by-type] [--pipeline {bm25,full}]` | per-instance 隔离评分，输出 recall@1/5/10、MRR，可按问题类型拆分，`full` 走真实 FTS5+fusion 检索栈。 |
| `mcp serve` | 启动 MCP stdio server；加 `--sse` 使用 SSE。 |
| `doctor --scope all` | 检查依赖、模板、store、FTS5、索引和可选组件状态。 |
| `codex-prep TASK` | 生成给 Codex 的 prompt 前缀。 |
| `codex-ingest --commit` | 从 stdin 解析 `**新发现:**` / `**Findings:**` 并写入记忆。 |
| `distill --transcript PATH \| --stdin --commit` | 从会话 transcript 抽取记忆；默认 dry-run，加 `--commit` 写入。 |

查看任意命令的完整参数：

```bash
python3 -m mnemosyne <command> --help
```

## 配置

项目配置位于 `.mnemosyne/config.toml`：

```toml
[thresholds]
decay_per_run = 1
bonus_access = 5
bonus_recall = 20
core_strength = 80
core_access_count = 3
archive_strength = 30
deprecated_strength = 5

[memory]
types = ['arch_decision', 'pitfall', 'codebase', 'preference', 'handoff']

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
dimensions = 384

[rerank]
enabled = false
backend = "cross_encoder"
model = "BAAI/bge-reranker-base"
onnx_path = ""
top_n = 5

[distill]
enabled = false
engine = "heuristic"
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
link_expansion_max_hops = 1

[relations]
allow_custom = false

[mcp]
default_search_limit = 5

[mcp.sse]
enabled = false
host = "127.0.0.1"
port = 3700
```

说明：

- `thresholds` 控制记忆衰减、召回和晋升阈值。
- `memory.types` 是允许的记忆类型清单。写入其它类型会提示 warning。
- `injection.max_tokens` 控制 hooks 注入记忆的近似 token 上限。
- `injection.summary_chars` 控制注入摘要的单条截断长度（默认 120）。注入是"目录"而非全文：每条记忆一行，完整内容用 `python3 -m mnemosyne show ID` 按需获取。同一会话内已注入过的记忆不会重复注入。
- `search.index_enabled = false` 可关闭 SQLite FTS5，强制使用内存 BM25。
- `embedding.enabled` 与 `rerank.enabled` 默认关闭，基础安装不需要额外依赖。
- `distill.enabled` 默认关闭（opt-in）；启用后由 Stop hook / `codex-ingest` / Hermes `on_session_end` 触发自动记忆形成。`engine` 可选 `heuristic`（默认，stdlib 启发式）、`llm`（需配置 `[distill.llm]` 的 backend/model/api_base/api_key_env）或 `host`（解析 agent 输出的 `**新发现:**` 块）。`confidence_threshold` 过滤低置信度候选，`max_findings_per_session` 限制单次会话写入条数，`dedup_threshold`/`subject_threshold` 控制写入前的去重与 supersede 判定。`heuristic` 引擎为高精度设计：pitfall 仅在较短、错误与修复标记相邻、且非分步指令式的 turn 上触发，避免把长篇对话解释误当记忆。
- `fusion.link_expansion` 控制 typed links 是否参与召回扩展。
- `relations.allow_custom` 控制 `link` 是否默认接受非预定义关系。
- `mcp.sse` 控制可选 SSE 地址；stdio 始终是 `mcp serve` 默认值。

全局 store 默认在 `~/.mnemosyne/`，可以覆盖：

```bash
export MNEMOSYNE_HOME=/path/to/custom/store
```

## 搜索索引

每个 store 下会维护一个 `index.sqlite`：

```text
~/.mnemosyne/index.sqlite
your-project/.mnemosyne/index.sqlite
```

写入、链接、搜索命中回写时会尽量增量更新索引。若文件被手动编辑或索引异常，可以重建：

```bash
python3 -m mnemosyne reindex --scope all
```

`search --format json` 会输出 `why_matched` 和 `score_breakdown`，方便区分 BM25、
vector、link boost 与 reranker 的贡献。切换 embedding 模型后，运行：

```bash
python3 -m mnemosyne embed-backfill --scope all
```

## 文件格式

记忆文件是 Markdown，头部是 YAML frontmatter：

```markdown
---
id: pitfall-2026-05-28-0c0a59
type: pitfall
source: codex
strength: 80
created: 2026-05-28
last_accessed: 2026-05-28
access_count: 2
tags: [auth, xss, security]
links:
  - id: arch_decision-2026-05-28-9a8341
    rel: caused_by
canonical_summary: "JWT XSS 漏洞: localStorage 会暴露 token"
injection_summary: "JWT XSS 漏洞: localStorage 会暴露 token，改 httpOnly cookie + CSRF。"
status: active
expires: 认证方案重构时失效
---

## JWT XSS 漏洞

localStorage 中的 token 会被 XSS 读取，改用 httpOnly cookie 并补 CSRF 防护。
```

`source` 用来追溯写入方，如 `claude-code`、`codex`、`user`。需要查看完整字段时用
`python3 -m mnemosyne show ID`。

`expires` 支持两种写法：ISO 日期（`2026-12-31`，maintain 到期自动归档）或自由文本
条件注记（如 `认证方案重构时失效`，仅供人和 agent 阅读，不参与自动归档）。

## 存储结构

```text
~/.mnemosyne/              # 全局记忆
  core.md
  config.toml
  working/
  archive/YYYY-MM/
  index.sqlite              # 按需生成
  MEMORY.md                 # 按需生成

your-project/.mnemosyne/   # 项目记忆
  core.md
  config.toml
  working/
  archive/YYYY-MM/
  index.sqlite              # 按需生成
  MEMORY.md                 # 按需生成
  .lock                     # maintain 时生成
  .last_maintain            # SessionStart 节流维护时生成
```

并发安全：

- Markdown 写入使用临时文件 + `os.replace` 原子替换。
- 维护任务使用 `portalocker` 加锁。
- SQLite 索引使用 WAL 和 `busy_timeout`，适合 Claude Code、Codex 与 Hermes 同时读写。

## 排障

先跑：

```bash
python3 -m mnemosyne doctor --scope all
```

常见情况：

| 现象 | 处理 |
|---|---|
| Claude hook 没输出 | 确认 `templates/settings.json` 已合并，且里面的 `python` 在 Claude Code 环境可用。 |
| Codex 没自动读记忆 | 确认 `~/.codex/AGENTS.md` 或项目 `AGENTS.md` 中有 Mnemosyne 指令。 |
| 搜索结果旧或缺失 | 跑 `python3 -m mnemosyne reindex --scope all`。 |
| FTS5 不可用 | `doctor` 会提示回退到内存 BM25；换带 SQLite FTS5 的 Python 可恢复持久索引。 |
| `mcp serve` 提示缺依赖 | 安装 `python3 -m pip install -e ".[mcp]"`。 |
| 切换 embedding 模型后向量未命中 | 跑 `python3 -m mnemosyne embed-backfill --scope all`。 |
| 不想某项目自动生成 `.mnemosyne/` | 在项目根创建 `.mnemosyne-disable`。 |
| 同一会话里记忆没有再次注入 | 会话级去重的预期行为；需要全文时用 `python3 -m mnemosyne show ID`。 |
| `codex-ingest` 没写入 | 确认传入文本有 `**新发现:**` 块，并且命令带了 `--commit`。 |
| Hermes provider 不生效 | 确认已运行 `install-hermes` 且重启了 Hermes；用 `--dry-run` 预览安装内容。 |

## 更新日志

详见 [CHANGELOG.md](CHANGELOG.md)。当前版本 0.4.0：新增会话级注入去重、单行注入 + `show` 按需拉全文（progressive disclosure）、embedding 增量 backfill；注入排序改为相关性优先，link expansion 走 SQLite，maintain 对账重写 MEMORY.md。0.3.2 修复了全局库衰减随活跃项目数放大、rerank 分数刻度混排、损坏文件拖垮整库、`expires` 语义分裂四个问题。

## License

MIT
