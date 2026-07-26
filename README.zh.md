# Mnemosyne

[![CI](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml/badge.svg)](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml)

> 希腊记忆女神，缪斯九姐妹之母。

[English README → README.md](README.md)

**Mnemosyne 是一个本地优先、agent 无关的通用记忆内核。** 长期记忆保存为
你磁盘上的 Markdown 文件，由 SQLite FTS5 混合检索索引，被你使用的所有
agent 共享——Claude Code、Codex CLI、Cursor、Hermes，以及任何会说 MCP、
能跑 shell 命令或读文件的 agent。

它最适合解决这类问题：

- 不同 agent 之间缺少同一套长期记忆，每换一个工具就失忆一次。
- Agent 每次开新会话都要重新解释项目约束、踩坑和偏好。
- 项目记忆需要能被人直接查看、审计、编辑，而不是锁在某个数据库或云平台里。
- 希望全局偏好和项目知识分开保存，避免互相污染。

## 接入任意 agent

三条接入路径共享同一个存储与检索内核，按你的 agent 有什么能力来选：

1. **MCP**（Claude Code / Codex CLI / Cursor / Cline / Windsurf / Gemini CLI…）：
   `python3 -m mnemosyne mcp serve`，纯 stdlib 实现零依赖；工具面含
   `mnemosyne_search` / `mnemosyne_write` / `mnemosyne_read_core` /
   `mnemosyne_prep_context` 等。
2. **CLI + 注入事件**（任何有 shell 或 hooks 机制的 agent）：全部能力都是
   子命令；四个中立生命周期事件 `session_start` / `turn_start` /
   `file_touch` / `session_end` 通过 `mnemosyne inject --event <name>`
   提供自动注入与自动蒸馏，`--fail-safe` 保证任何错误都不阻塞宿主。
   适配器契约见 [docs/adapters.md](docs/adapters.md)。
3. **直接读写文件**（任何能读文件的 agent）：`.mnemosyne/core.md` 是常驻
   摘要，每条记忆一个 Markdown 文件；格式规范见
   [docs/interface.md](docs/interface.md) 与
   [docs/handoff-format.md](docs/handoff-format.md)。

官方适配器（同一内核上的平级实现）：Claude Code（hooks 确定性注入，
`mnemosyne install claude-code` 查看接入步骤）、Codex CLI（AGENTS.md +
prep/ingest 交接，`mnemosyne init --agent codex`）、Hermes（原生
MemoryProvider 插件，`mnemosyne install hermes`）。

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
| 通用注入事件 | `mnemosyne inject` 承载 session_start / turn_start / file_touch / session_end 四个中立事件，任何宿主的 hooks 都能映射。 |
| Claude Code 适配器 | hooks 协议壳映射到通用事件：SessionStart、UserPromptSubmit、PreToolUse、Stop。 |
| 交接通道 | `prep` / `ingest`（旧名 `codex-prep` / `codex-ingest` 保留），findings 块有 Markdown 与 JSON 双格式规范。 |
| Hermes 原生集成 | `install-hermes` 一键安装原生 MemoryProvider 插件，重启后自动注入与检索。 |
| 跨 agent 自动记忆形成 | 可选的 `distill` 引擎（heuristic/LLM/host）从会话 transcript 中增量抽取记忆，默认关闭，写入前会去重/supersede，findings 可带 `evidence` 原文溯源。 |
| LongMemEval 基准 | `eval convert/fetch longmemeval` 和 `eval run --longmemeval` 提供 per-instance recall/MRR 与按问题类型拆分；`eval run --min-recall` 可作 CI 回归门槛。 |
| Progressive disclosure 注入 | hooks 注入是单行目录 + `show` 提示，同一会话内已注入过的记忆不会重复注入。 |
| 时间有效性 | 被 `supersedes` 取代的记忆标记失效但不删除，默认检索会过滤，`--include-superseded` 可回看。 |
| 记忆整合 | `consolidate` 合并同类型近重复的 working 记忆，默认 dry-run 预览。 |
| CI | GitHub Actions 跑测试套件并对检索质量做 recall@5 回归门槛。 |

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
`prep_context`（旧名 `codex_prep` 保留为可调用别名）暴露为 MCP tools。默认传输是 stdio，适合本地编辑器和 Agent；
`mnemosyne mcp serve --sse` 可用于调试或远程接入。

MCP server 是纯 stdlib 实现，无需安装任何额外依赖即可直接
`mnemosyne mcp serve`。仓库内的 `mnemosyne/templates/mcp_clients/` 提供
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
| `session_summary` | LLM 蒸馏生成的会话级摘要（`[distill].session_summary = true` 时输出）。 |

## 让 Claude Code、Codex 和 Hermes 共享记忆

Mnemosyne 的共享方式很朴素：Claude Code 和 Codex 都读写同一套
`~/.mnemosyne/` 与项目 `.mnemosyne/` 文件。

### Claude Code Desktop

把 hooks 模板合并到 Claude Code 的全局或项目 settings：

```bash
cat mnemosyne/templates/settings.json
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

再把 `mnemosyne/templates/CLAUDE.md` 的规则加入你的 `~/.claude/CLAUDE.md`，Claude Code 就会在
遇到踩坑、架构决策、用户偏好、代码库知识或交接信息时主动写入记忆。

### Codex Desktop

Codex 没有 Claude Code hooks，但可以通过 `AGENTS.md` 约定在任务开始和结束时调用
Mnemosyne CLI。

推荐全局配置：

```bash
mkdir -p ~/.codex
[ -f ~/.codex/AGENTS.md ] || cp mnemosyne/templates/AGENTS.md ~/.codex/AGENTS.md
```

如果 `~/.codex/AGENTS.md` 已经存在，把 `mnemosyne/templates/AGENTS.md` 里的 Mnemosyne 规则合并进去即可。
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

MCP server 为纯 stdlib 实现，无需额外安装，直接选择客户端模板：

```bash
cat mnemosyne/templates/mcp_clients/cursor.json
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
| `write --type T --importance N ...` | 写入一条记忆。写入前自动查重：完全重复会跳过（`--allow-duplicate` 强写），同主题新结论会自动建 `supersedes` 链。 |
| `search QUERY --format json` | 搜索记忆；优先使用 SQLite FTS5，回退到内存 BM25。 |
| `show ID` | 查看一条完整记忆，包括 frontmatter。 |
| `link ID1 ID2 --rel REL` | 用 typed relation 链接两条记忆；自定义关系需 `--allow-custom`。 |
| `graph ID --format mermaid` | BFS 展开关系图，支持 `mermaid`、`ascii`、`json`。 |
| `maintain --dry-run` | 预览衰减、归档和 core 晋升候选。 |
| `consolidate [--commit]` | 合并近重复 working 记忆（同类型、相似度 ≥0.8）；默认 dry-run 预览。 |
| `maintain --scope all` | 维护全局和项目记忆。 |
| `reindex --scope all` | 全量重建搜索索引。 |
| `embed-backfill --scope all` | 为已有记忆计算或刷新 embedding。 |
| `eval run --corpus FILE` | 输出固定语料的 recall、MRR 和延迟基线。 |
| `eval convert longmemeval --raw FILE --out DIR` | 把 LongMemEval 原始数据转换成 seed memories + corpus。 |
| `eval fetch longmemeval --variant {s,m}` | 下载 LongMemEval 数据集（官方地址未确认前会提示手动下载）。 |
| `eval run --longmemeval [--by-type] [--pipeline {bm25,full}]` | per-instance 隔离评分，输出 recall@1/5/10、MRR，可按问题类型拆分，`full` 走真实 FTS5+fusion 检索栈。 |
| `mcp serve` | 启动 MCP stdio server；加 `--sse` 使用 SSE。 |
| `doctor --scope all` | 检查依赖、模板、store、FTS5、索引和可选组件状态。 |
| `prep TASK`（旧名 `codex-prep`） | 生成给任意 agent 的 prompt 前缀。 |
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

# 以下三类键只从全局 ~/.mnemosyne/config.toml 读取，项目内 config.toml 里会被忽略：
# api_base、api_key_env、onnx_path（以及 distill.llm.backend）。
# 原因见「配置的信任边界」一节。
[embedding]
enabled = false
backend = "onnx"
model = "BAAI/bge-small-zh-v1.5"
dimensions = 384
batch_size = 32

[rerank]
enabled = false
backend = "cross_encoder"
model = "BAAI/bge-reranker-base"
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
model = ""

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
host = "127.0.0.1"
port = 3700
```

说明：

- `thresholds` 控制记忆衰减、召回和晋升阈值；`bonus_write` 是写入时的强度加成（当前仅供未来场景预留，写路径尚未使用）。
- `memory.types` 是允许的记忆类型清单。写入其它类型会提示 warning。
- `injection.max_tokens` 控制 hooks 注入记忆的近似 token 上限。
- `injection.summary_chars` 控制注入摘要的单条截断长度（默认 120）。注入是"目录"而非全文：每条记忆一行，完整内容用 `python3 -m mnemosyne show ID` 按需获取。同一会话内已注入过的记忆不会重复注入。
- `search.index_enabled = false` 可关闭 SQLite FTS5，强制使用内存 BM25。
- `embedding.enabled` 与 `rerank.enabled` 默认关闭，基础安装不需要额外依赖；`embedding.batch_size` 控制 backfill 每批嵌入的记忆数，每批独立超时。
- `distill.enabled` 默认关闭（opt-in）；启用后由 Stop hook / `codex-ingest` / Hermes `on_session_end` 触发自动记忆形成。`engine` 可选 `heuristic`（默认，stdlib 启发式）、`llm`（需在**全局** config 配置 `[distill.llm]` 的 backend/api_base/api_key_env，见下方「配置的信任边界」）或 `host`（解析 agent 输出的 `**新发现:**` 块）。`confidence_threshold` 过滤低置信度候选，`max_findings_per_session` 限制单次会话写入条数，`dedup_threshold`/`subject_threshold` 控制写入前的去重与 supersede 判定。`heuristic` 引擎为高精度设计：pitfall 仅在较短、错误与修复标记相邻、且非分步指令式的 turn 上触发，避免把长篇对话解释误当记忆。`distill.session_summary` 开启后（需 `engine = "llm"`），每次蒸馏额外产出一条 `session_summary` 会话摘要；LLM findings 会带 `evidence` 原文引用便于溯源。Stop hook 的蒸馏是增量的：按 transcript 记录已处理轮次（`.distill_state.json`），每轮只处理新增内容。
- `fusion.link_expansion` 控制 typed links 是否参与召回扩展；`link_expansion_decay_fallback` 是多跳扩展时每跳的衰减系数；`bm25_pool_size`/`vec_pool_size` 控制各检索通路在融合前各自召回的候选池大小。
- `relations.allow_custom` 控制 `link` 是否默认接受非预定义关系。
- `mcp.expose_global`/`mcp.expose_project` 控制 MCP server 是否暴露对应作用域；`mcp.sse` 控制可选 SSE 地址，stdio 始终是 `mcp serve` 默认值。

### 配置的信任边界

项目内的 `.mnemosyne/config.toml` 是跟着仓库走的文件——clone 别人的仓库就等于加载了对方写的配置。因此以下这些"决定数据发往哪里、读哪个密钥、加载哪个模型文件"的键**只从全局 `~/.mnemosyne/config.toml` 读取**：

| 键 | 为什么只信全局 |
|---|---|
| `distill.llm.api_base`、`distill.llm.backend` | 否则一个恶意仓库可以把整段会话 transcript 发往任意主机 |
| `distill.llm.api_key_env`、`embedding.api_key_env` | 否则可以指定任意环境变量（如云厂商密钥）作为 Bearer token 送出 |
| `embedding.api_base` | 同上，向量化会把记忆正文发往该地址 |
| `embedding.onnx_path`、`rerank.onnx_path` | 否则可以指向仓库内的任意文件作为模型加载 |

项目 config 里出现这些键时会被忽略；值与默认不同（即像是被人刻意改过）时还会向 stderr 打一条提示。其余配置（含 `distill.enabled` 与 `distill.engine`）仍可按项目设置。

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

0.6.1 的索引格式为 v3。日常手工编辑无需主动重建：增量同步会对账文件路径和
frontmatter `id`，清理损坏文件或历史重复行；文件改名但 `id` 不变时会保留同一索引记录。
索引还会持久化 `status`，因此 superseded 过滤会在 BM25 候选池截断前执行，不会让失效
记忆挤掉仍然有效的结果。

被 `supersedes` 取代的记忆默认不出现在搜索结果中（失效不删除），用
`search --include-superseded` 回看历史结论；旧记忆 frontmatter 带
`invalidated_by` 指向取代者。

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
  .last_maintain            # SessionStart 节流维护时生成（按 store 各自计时）
  .session_injected.json    # 会话级注入去重状态，48h TTL 自动清理
  .distill_state.json       # 蒸馏增量处理状态（记录已处理的 transcript 轮次），7 天 TTL
```

并发安全：

- Markdown 写入使用临时文件 + `os.replace` 原子替换。
- 搜索访问回写会在 store 锁内重新读取最新文件，只修改访问字段，避免覆盖并发新增的关系或状态。
- finding 的分类、创建与 supersede 关联属于同一 store 事务；并发更新同一主题只留下一个 active head。
- 跨 store 操作按稳定路径顺序获取 `portalocker` 锁；SessionStart 维护调度用独立原子节流锁。
- SQLite 索引使用 WAL 和 `busy_timeout`，适合 Claude Code、Codex 与 Hermes 同时读写。

## 排障

先跑：

```bash
python3 -m mnemosyne doctor --scope all
```

常见情况：

| 现象 | 处理 |
|---|---|
| Claude hook 没输出 | 确认 `mnemosyne/templates/settings.json` 已合并，且里面的 `python` 在 Claude Code 环境可用。 |
| Codex 没自动读记忆 | 确认 `~/.codex/AGENTS.md` 或项目 `AGENTS.md` 中有 Mnemosyne 指令。 |
| 搜索结果旧或缺失 | 跑 `python3 -m mnemosyne reindex --scope all`。 |
| FTS5 不可用 | `doctor` 会提示回退到内存 BM25；换带 SQLite FTS5 的 Python 可恢复持久索引。 |
| 切换 embedding 模型后向量未命中 | 跑 `python3 -m mnemosyne embed-backfill --scope all`。 |
| 不想某项目自动生成 `.mnemosyne/` | 在项目根创建 `.mnemosyne-disable`。 |
| 同一会话里记忆没有再次注入 | 会话级去重的预期行为；需要全文时用 `python3 -m mnemosyne show ID`。 |
| `codex-ingest` 没写入 | 确认传入文本有 `**新发现:**` 块，并且命令带了 `--commit`。 |
| Hermes provider 不生效 | 确认已运行 `install-hermes` 且重启了 Hermes；用 `--dry-run` 预览安装内容。 |

## 更新日志

详见 [CHANGELOG.md](CHANGELOG.md)。当前版本 0.7.0 是「通用记忆内核」重构版：稳定 `mnemosyne.api`、中立注入事件（`mnemosyne inject`）、JSON findings 变体、transcript 解析注册表、平级适配器与分目录模板、英文文档主导；存储格式不变，所有旧命令/模块路径/MCP 工具名经 alias 继续可用。0.6.2 是安全与检索修复版本：PreToolUse hook 不再输出 `permissionDecision: allow`（此前会绕过 Edit/Write 的权限确认）；`api_base`/`api_key_env` 等网络配置改为仅信任全局 store，项目内 config.toml 不能再指定外传端点与凭证环境变量；修复中文查询在 FTS 通道恒零命中导致排序退化为按 strength、混合中英查询静默丢弃中文词、链接扩展加分无上限把 hub 记忆顶到第一；`templates/` 移入包内避免污染 site-packages，补 LICENSE 与打包元数据；CI 增加走真实检索管线的 recall 门槛。0.6.1 关闭了并发写回、维护调度与 supersede 事务竞态并统一各入口的去重与作用域边界。

## License

MIT
