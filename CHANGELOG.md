# 更新日志

本文件记录 Mnemosyne 的重要变更，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本遵循语义化版本。

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
