# 更新日志

本文件记录 Mnemosyne 的重要变更，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本遵循语义化版本。

## [0.6.2] - 2026-07-26

### 安全 (Security)

- **PreToolUse hook 不再放行工具调用**：该 hook 只负责注入记忆，此前输出的 `permissionDecision: "allow"` 会绕过 Claude Code 对 Edit/Write 的权限确认——只要目标文件名检索命中任意记忆（弱 token 如 `py` 也会命中），写入就被静默自动批准。现在只输出 `additionalContext`。
- **网络相关配置仅信任全局 store**：`api_base`、`api_key_env`、`distill.llm.backend`、`embedding.onnx_path`、`rerank.onnx_path` 只从 `~/.mnemosyne/config.toml` 读取。此前项目内 `.mnemosyne/config.toml` 可指定任意端点与任意环境变量作为 Bearer token，clone 一个恶意仓库即可在会话结束时把环境变量与整段 transcript 外传。项目配置中出现这些键时会被忽略并在值非默认时告警。

### 修复 (Fixed)

- **中文检索排序失效**：FTS5 trigram tokenizer 无法匹配少于 3 字符的查询 token，而查询构造把中文切成 2 字 bigram，导致纯中文查询 MATCH 恒为 0 命中、排序退化为按 strength，持久索引反而比内存 BM25 更差。查询构造改为对 ≥3 字的连续中文串发滑窗 trigram 加整段 phrase（作强信号），1-2 字串交给 LIKE 通道。
- **混合中英查询丢弃中文词**：LIKE 兜底此前以「FTS 零命中」为触发条件，混合查询中英文 token 一旦命中，中文部分即被静默忽略。现在两条通道的结果按路径合并、分数相加。
- **LIKE 通道无相关性信号**：此前仅按 strength 排序，高 strength 的片段命中会压过完整命中；改为按命中 query token 数排序，strength 仅作 tie-break。
- **链接扩展加分失控**：单个目标的累计 link boost 现在以最佳直接命中的 0.8 倍为上限，且只从 top-10 候选出发扩展（此前遍历整个 50 候选池并无上限累加，多个中游候选可把一个与查询无关的 hub 记忆推到第一）。`supersedes` 不再给被取代的旧记忆加分（也不再把它拉进结果），`contradicts` 加分降为 0 但保留矛盾标注。
- **文件末尾无换行时 frontmatter 整体丢失**：`split_frontmatter` 现在也接受作为最后一行、无结尾换行的 `---`。此前少一个换行就会让整条记忆的 id 变空，进而从索引与检索中消失。
- **并发归档使 search 崩溃**：`search` 的访问加分路径此前只抑制 `LockException`，另一进程的 `maintain` 归档该文件时抛出的 `FileNotFoundError` 会让整个命令带 traceback 退出、已检索结果全部丢失。
- **write_memory 释放锁后删除锁文件**：这会破坏互斥（等待者持旧 inode、新来者建新 inode，两者同时进入临界区并争用同一 tmp 路径）。锁文件现在常驻，与 `lock_store` 策略一致。
- **cross-encoder 输入被截空**：`max_length` 从 64 提到 256，并给 query 设独立上限（64），此前较长的查询会把文档配额压到只剩摘要开头几个词，rerank 分数接近噪声却无条件覆盖融合分数。
- **tokenizer 覆盖面**：新增假名、谚文、CJK 扩展 A 与兼容表意文字，以及带重音的拉丁字母；此前这些内容写入后永久不可检索。
- **LongMemEval abstention 拉低指标**：转换时跳过无证据 session 的 `*_abs` 实例，此前它们恒计 recall/MRR = 0，使整体数字与公开口径不可比。

### 变更 (Changed)

- **`templates/` 移入包内**：改为 `mnemosyne/templates/`，通过包内相对路径定位。此前作为 wheel 顶层目录安装到 site-packages 根下，会与任何同名顶层目录的包冲突，卸载对方时会连带删除本项目模板。
- **打包元数据补全**：新增 LICENSE 文件（README 早已声明 MIT）、`readme`/`license`/`authors`/`classifiers`/`urls`，以及 `dev` extra（pytest）。
- **CI**：新增走真实管线（SQLite FTS5 + fusion）的 recall@5 门槛——此前唯一的门槛只测内存 BM25，上述检索缺陷全部存在时它依然全绿；另加 macOS job、`ruff check`（仅 E9/F 规则）、optional extras 的安装与导入验证、pip 缓存。

## [0.6.1] - 2026-07-18

### 修复 (Fixed)

- 搜索访问计数在 store 锁内重读最新文件后仅更新访问字段，避免覆盖并发新增的 links、`status`、`invalidated_by` 或正文；跨 store 关系写入按稳定顺序加锁。
- finding 的分类、创建与 supersede 关联成为单一 store 事务；并发同主题更新只保留一个 active head。SessionStart 维护调度增加原子节流，避免重复衰减。
- finding 去重限定在目标作用域及相同类型，并由 CLI、Codex ingest、distill 与 MCP 共同执行；重复 Codex/MCP 写入返回既有 ID。
- MCP 的 search/read/show/write/link/graph/maintain/Codex prep 全部执行 `expose_global` / `expose_project`，关闭的作用域不会被读取或修改。
- 混合检索在 BM25/vector 候选池截断前过滤 superseded，并在向量与链接扩展后再次执行类型、归档和状态过滤，失效高分结果不再挤掉 active 命中。
- `consolidate --commit` 保守拒绝冲突元数据，兼容合并保留证据、生命周期、访问统计及链接，并重写跨作用域外部反向引用。
- FTS 索引升级至 v3 并持久化 `status`；增量同步跳过损坏文件、清理历史重复行，正确处理 frontmatter ID 修改以及同 ID 文件改名。
- 包、运行时、MCP 与 Hermes 插件版本统一由 `mnemosyne.__version__` 驱动。

## [0.6.0] - 2026-07-04

### 新增 (Added)

- **失效不删除（superseded 检索过滤）**：被 `supersedes` 取代的记忆标记 `status = "superseded"` 并写入 `invalidated_by` 指向取代者；默认检索不再返回，`search --include-superseded` 可回看历史结论。`write` 自动 supersede 与手动 `link --rel supersedes` 两条路径行为一致。
- **`consolidate` 整合命令**（sleep-time 式维护的保守 v1）：合并同类型、相似度 ≥ 阈值（默认 0.8）的近重复 working 记忆——弱者并入强者（strength 取 max、tags/body 合并）、删除弱者文件与索引行、重写 MEMORY.md。默认 dry-run，`--commit` 生效；纯启发式，无 LLM 依赖。
- **检索质量 CI 门槛**：`eval run --min-recall X` 在固定 50 条语料的 recall@5 低于阈值时以非零码退出；新增 GitHub Actions workflow（Python 3.11-3.13 跑 pytest + `--min-recall 0.95` 回归门槛）。

## [0.5.0] - 2026-07-04

### 新增 (Added)

- **蒸馏增量化**：Stop hook 按 transcript 记录已处理轮次（store 根 `.distill_state.json`，7 天 TTL），每次只蒸馏新增轮次；transcript 轮换自动从头处理。此前每轮回复结束都全量重蒸馏整个会话。
- **Finding 溯源（evidence）**：`Finding` 新增可选 `evidence` 字段；host 引擎的 `**新发现:**` 块支持 `- evidence:` 行；LLM 引擎要求模型给出对话原文短引用；持久化到记忆 frontmatter 的 `evidence` 键。
- **会话摘要类型（session_summary）**：新记忆类型；`[distill].session_summary = true` 且 `engine = "llm"` 时，每次蒸馏额外产出一条会话级摘要。

### 变更 (Changed)

- **write 统一查重**：`write` 在所有路径（含 `--force`）先做 duplicate/supersede 判定：完全重复跳过写入并提示既有 ID（新 flag `--allow-duplicate` 强写）；同主题新结论自动写入并建 `supersedes` 链、降级旧记忆。"先 search 再写" 从约定变成机制。

## [0.4.0] - 2026-07-04

### 新增 (Added)

- **会话级注入去重**：同一会话内已注入过的记忆不再重复注入。状态存于 store 根目录 `.session_injected.json`（按 hook 事件的 `session_id` 记账，48 小时 TTL 自动清理），UserPromptSubmit 与 PreToolUse 两个注入点均生效。
- **注入瘦身（progressive disclosure）**：注入从"两行 + 220 字摘要"改为"单行目录条目 + `show` 提示"，新增 `injection.summary_chars` 配置（默认 120）。注入是目录不是全文，agent 需要细节时用 `python3 -m mnemosyne show ID` 按需拉取。
- **embedding 增量 backfill**：`embed-backfill` / `reindex` 只重嵌"缺失 / 换模型 / 内容已更新"的记忆（按 `embedding_mtime` 与文件 mtime 比对），按 `embedding.batch_size` 分批、每批独立超时，失败批打 stderr 警告——此前是全量重嵌 + 整批 30s 超时静默失败。

### 变更 (Changed)

- **注入排序改为相关性优先**：`format_for_injection` 此前按 strength 主导排序，强但不相关的记忆会挤掉弱但相关的；现在检索分主导、strength 仅作平分决胜。
- **link expansion 走 SQLite**：链接目标解析改查 `memories_meta` 表（带单次检索内 memoize），不再对每条链接线性重读整库文件；索引不可用时自动回退线性扫描。
- **MEMORY.md 对账重写**：`maintain`（非 dry-run）时按 working 区现存记忆全量重写 MEMORY.md，归档/取代条目不再永久残留；写入路径仍为廉价追加。

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
