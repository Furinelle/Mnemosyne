# 更新日志

本文件记录 Mnemosyne 的重要变更，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本遵循语义化版本。

## [0.3.2] - 2026-07-04

### 修复 (Fixed)

- **全局库衰减速率随活跃项目数放大**：SessionStart 的 maintain 节流标记 `.last_maintain` 此前存在项目库、却触发 `maintain --scope all`，一天内在 N 个项目开会话会让全局库被 decay N 次。现改为按 store 各自存标记、各自节流触发（`--scope project` / `--scope global`），全局库每个间隔期只衰减一次；且没有项目库时全局库也能得到维护。
- **rerank 启用时分数刻度混用**：`_rerank` 只覆写 top `2N` 条的分数为 reranker 分值（0~1），尾部保留 BM25/RRF 原始分后全体混排，未重排的尾部会反超重排头部。现在重排区整体排在未重排尾部之前。
- **单个损坏文件拖垮整库**：手工编辑坏的 frontmatter（如 `strength: high`）此前抛 `ValueError` 使 search/maintain/hooks 全部失效。现在非数字计数字段回退 0，无法解码/解析的文件被跳过；`doctor` 新增 per-store 坏文件报告。
- **`expires` 语义分裂**：文档示例是自由文本、lifecycle 却按 ISO 日期做字符串比较（不误归档纯靠字典序运气）。现仅 `YYYY-MM-DD` 格式参与到期归档，其余值保留为注记；`write --expires` 传非日期值时输出警告。

## [0.3.1] - 2026-06-19

### 修复 (Fixed)

- **distill 去重失效导致重复写入**：`classify_against_store` 此前用新 finding 的完整文本去和库中记忆的**截断 `injection_summary`**（约 220 字）算 Jaccard 相似度，长内容因摘要丢失尾部 token 永远达不到 `dedup_threshold`，于是同一条记忆每次会话被重复写入（实测全局库出现 45 条重复）。现改为遍历 top-k 候选并比对**完整 body**。
- **全局库被误当项目库写入**：`find_project_store` 在 cwd 为 `$HOME`（且 `~/.mnemosyne` 即全局库）时会把全局库目录当作"项目库"返回，导致项目作用域的写入静默污染全局库。现在 `find_project_store` 会跳过全局根目录；`write_finding` 在没有真实项目时显式回退到 `global_store()`（正确的 global 作用域）。
- **启发式抽取精度过低**：pitfall 规则此前只要 assistant turn 中**任意位置**同时出现"错误"类与"修复"类词就捕获，导致长篇 SSH/permission 解释和分步指令被当作 pitfall 写入（清理出的垃圾绝大多数来自此）。现要求：turn 较短（≤280 字）、错误与修复标记**相邻**（间距 ≤50 字）、且非"第 N 步/阶段 N"指令式内容；preference 限制在 ≤200 字。
- **`__version__` 与 `pyproject` 不同步**：`mnemosyne/__init__.py` 的 `__version__` 此前停留在 `0.2.0`（0.3.0 bump 时漏改），现同步到 `0.3.1`。

## [0.3.0] - 2026-06-16

### 新增 (Added)

- **跨 agent 自动记忆形成（`distill`）**：可选能力，默认关闭。新增 `mnemosyne/distill/` 核心，支持启发式（stdlib，默认）、可选 LLM（`engine = "llm"`，需配置 API key）和 host（解析 agent 输出的 `**新发现:**` 块）三种引擎；写入前会经过去重/supersede 护栏，避免重复或过时记忆入库。新增 `mnemosyne distill` 子命令（`--transcript PATH | --stdin`、`--source`、`--commit`；默认 dry-run，不加 `--commit` 不会写入）。项目 `config.toml` 新增 `[distill]` 配置块（`enabled`、`engine`、`confidence_threshold`、`max_findings_per_session`、`dedup_threshold`、`subject_threshold`，以及 `[distill.llm]` 子表）。三处触发点：Claude Code 的 Stop hook 在 `[distill].enabled = true` 时自动对会话 transcript 执行 distill（已防御 `stop_hook_active` 重入）；Codex 通过 `**新发现:**` 配合 `codex-ingest --commit`；Hermes 通过 provider 的 `on_session_end`。新增可选 extra `mnemosyne[distill]`。
- **LongMemEval 检索基准**：新增 `mnemosyne eval convert longmemeval --raw FILE --out DIR`（适配器 `mnemosyne/eval/adapters/longmemeval.py`）和 `mnemosyne eval fetch longmemeval --variant {s,m}`（下载器；官方 URL 目前仍是占位符，会以清晰的报错提示手动下载）。`mnemosyne eval run --longmemeval [--by-type] [--pipeline {bm25,full}]` 输出 per-instance 隔离评分的 recall@1/5/10 与 MRR，支持按问题类型拆分；`full` pipeline 会通过真实的 FTS5 + fusion 检索栈打分。`EvalItem` 新增可选字段 `instance_id`、`question_type`。

### 变更 (Changed)

- Claude Code / Codex 模板中的默认命令统一为 `python3 -m mnemosyne...`，避免没有 `python` shim 的 macOS 环境直接复制模板后 hooks 失效。

## [0.2.1] - 2026-06-07

### 修复 (Fixed)

- **混合检索向量保活**：`index_memory` 由 `INSERT OR REPLACE` 改为 UPSERT，搜索访问刷新元数据时不再清空 `embedding` 等列；`update_memory_index` 改为只刷新 FTS 行。此前 backfill 之后第一次搜索就会抹掉命中记忆的向量，使混合检索退化为纯 BM25。
- **`decode_embedding` 崩溃防护**：维度与 blob 字节长度不匹配时返回空向量，避免 `struct.error` 在 `reindex` / 向量检索时未捕获崩溃。
- **负 importance 钳制**：`write --importance` 传负值现在钳制到 0，与 Codex 写入路径行为一致（此前可写入负强度）。
- **记忆 ID 抗碰撞**：`make_memory_id` 后缀由 6 位随机十六进制改为 8 位 uuid4，降低同日大量写入时的静默覆盖风险。
- **frontmatter 转义 round-trip**：含特殊字符的标量值序列化时先转义反斜杠再转义引号，读取时反转义，修复带 `\` 或 `"` 的值无法往返的损坏问题。
- **CJK token 估算**：注入预算按 CJK 字符加权（约 0.6 token/字），修正中文 / 日文 / 韩文约 2.5x 的低估，避免静默超出注入上限被截断。
- **`expires` 字段生效**：维护时过期记忆会被归档（无视强度）。此前 `--expires` 只写入不读取，过期记忆会被永久注入。

### 变更 (Changed)

- `link --rel supersedes` 现在会降低被取代记忆的强度（激活 `demote_target` 语义）。

### 移除 (Removed)

- `mnemosyne eval compare` 子命令：原实现是无操作（两次运行相同评测，delta 恒为 0，且配置从未应用到检索引擎），已移除。请使用 `mnemosyne eval run`，它会对比 legacy 与 bigram tokenizer 的真实 recall 差异。

## [0.2.0]

- 检索质量三轨升级：CJK bigram、可插拔 embedder、RRF 融合、cross-encoder rerank、eval harness。
- MCP 服务化：通过 stdio / 可选 SSE 暴露记忆工具。
- 关系图谱：typed links、关系权重扩展，以及 Mermaid / ASCII / JSON 输出。
- Hermes 原生 MemoryProvider 集成。
