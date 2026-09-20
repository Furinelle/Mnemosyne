# 参考来源与复用边界

检查日期：2026-09-19。S 为已固定提交的 Mnemosyne 源码/记录；E 为公开外部设计参考。外部主分支会变化，代码级实现前再锁定 commit 和许可，不根据 Star 或宣传 benchmark 排定任务。没有复制外部实现代码。

## S01 固定 master 基线

来源：<https://github.com/Furinelle/Mnemosyne/commit/d05f02853f42771ae9ca04d78b79739d01f1a76c>

用途与边界：GitHub branches/master 返回的提交；2026-09-19T12:07:54Z。

## S02 Cargo.toml

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/Cargo.toml>

用途与边界：version=1.0.0；edition=2024；rust-version=1.95；optional onnx。

## S03 api.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs>

用途与边界：已保守精确去重；duplicate_entry 未比较 source；maintain 仍 per_run；consolidate 仅精确等价自动提交。

## S04 schema / ingest

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/schema.rs>

用途与边界：Memory.extra 与受限 frontmatter；src/ingest.rs 的 Finding 仍为 type/title/content/evidence 等基础字段。

## S05 context.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs>

用途与边界：已有整包 estimated budget/BUDGET_TOO_SMALL；host/channel/session 键；路径+scope+id 身份；file_touch 仍 basename。

## S06 vectors.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/vectors.rs>

用途与边界：内容哈希、fingerprint、dimension、锁外 compute+写回 CAS；坏向量读取过滤与回填 fresh 判断不一致。

## S07 models.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/models.rs>

用途与边界：fingerprint 包括配置、本地模型/词表字节和输入版本；HTTP/ONNX 可选。

## S08 search.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/search.rs>

用途与边界：rust-index.sqlite，mtime_ns/size 增量同步，store lock+恢复；现有检索路径不是旧 Python fusion.py。

## S09 mcp.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/mcp.rs>

用途与边界：八工具、project_path、JSON-RPC dispatch、固定声明 2024-11-05、SSE 上限。

## S10 main / native migration

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs>

用途与边界：现有 CLI 命令；尚无 history/checkpoint/proposals/sleep；原生边界见 docs/rust-migration.md。

## S11 relations.rs

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/relations.rs>

用途与边界：可恢复两文件/跨 store 关系 journal 和 coordinator；不是持久语义历史。

## S12 rust-migration.md

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-migration.md>

用途与边界：Python/Hermes 退役、native aliases、Markdown 与双 SQLite cache、新旧 writer 限制。

## S13 rust-local-cutover.md

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-local-cutover.md>

用途与边界：阶段性本机记录；真实 ONNX、原生 44 测试的记录不应移植成后来版本的实跑证明。

## S14 CI workflow

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/.github/workflows/ci.yml>

用途与边界：Linux/macOS matrix；fmt/clippy/tests/mocks/三检索门禁/onnx release build。

## S15 固定提交托管 CI

来源：<https://github.com/Furinelle/Mnemosyne/actions/runs/35441992545>

用途与边界：核查时 completed/success；不是本轮 assistant 本机执行结果。

## S16 旧接口文档残留

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/interface.md>

用途与边界：仍描述 Python extra API 与 index.sqlite，应与 native migration 对齐。

## S17 仓库 agent 规则

来源：<https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/AGENTS.md>

用途与边界：以当前源码测试为准，只保存经过核验、可复用且不敏感的长期发现。

## E01 Mem0

来源：<https://github.com/mem0ai/mem0>

用途与边界：参考多层记忆、追加事实；README 的托管分数不能直接代表开源或本项目效果。

## E02 Graphiti

来源：<https://github.com/getzep/graphiti>

用途与边界：参考来源和时态；不移植图数据库。

## E03 Letta / MemFS

来源：<https://github.com/letta-ai/letta>

用途与边界：参考文件上下文/状态记忆；实施时核对实际维护入口，不复制运行框架。

## E04 Basic Memory

来源：<https://github.com/basicmachines-co/basic-memory>

用途与边界：参考 Markdown、人机共同编辑、MCP 项目上下文；独立实现。

## E05 claude-mem

来源：<https://github.com/thedotmack/claude-mem>

用途与边界：参考 search→timeline→按需批量详情；不默认完整会话采集。

## E06 Hindsight

来源：<https://github.com/vectorize-io/hindsight>

用途与边界：参考带来源的归纳知识；不将模型设为基本运行前提。

## E07 Memvid

来源：<https://github.com/memvid/memvid>

用途与边界：参考持久模型一致性与便携容器；不将 Markdown 改为二进制真源。

## E08 agent-memory

来源：<https://github.com/tigerless-labs/agent-memory>

用途与边界：参考 sleep/report/proposal；本计划采用更保守语义审批和显式 host request/import。

## E09 ai-memory temporal

来源：<https://github.com/akitaonrails/ai-memory/blob/main/docs/temporal.md>

用途与边界：参考 ingestion/system time，不冒充 world time；外部 GitHub 页面此次正文渲染有限，细节实现前需锁提交读文件。

## E10 OMEM

来源：<https://github.com/ourmem/omem>

用途与边界：参考七决策 reconciliation；不复制自动权限和云服务。

## E11 YantrikDB

来源：<https://github.com/yantrikos/yantrikdb>

用途与边界：README 限定结构化/识别的单值断言冲突，非任意自然语言矛盾检测。

## E12 MCP 2024-11-05 tools

来源：<https://modelcontextprotocol.io/specification/2024-11-05/server/tools>

用途与边界：匹配当前代码声明版本，核查工具业务错误与协议错误。

## E13 MCP 2024-11-05 lifecycle

来源：<https://modelcontextprotocol.io/specification/2024-11-05/basic/lifecycle>

用途与边界：核查初始化/协商；不以版本老本身认定缺陷。

## E14 LongMemEval

来源：<https://github.com/xiaowu0162/LongMemEval>

用途与边界：外部完整语料/评测参考；固定样本只作回归。

## 此轮外部核查深度

公开仓库主页/README：Mem0、Graphiti、Letta、Basic Memory、claude-mem、Hindsight、Memvid、tigerless-labs/agent-memory、OMEM、YantrikDB、LongMemEval。另打开 ai-memory temporal 的 GitHub 页面，但其本轮 HTML 正文可读性有限，因此历史设计在这里作为独立方案，不声称逐行复核了它的新实现。

MCP 使用当前 Mnemosyne 声明的 2024-11-05 tools/lifecycle 文档作对照，不强迫升级到最新协议；客户端需要更高版本时另做兼容矩阵。Tool annotations 不是权限校验。

本计划没有重用竞品的性能数字、没有审计竞品全部源码，也没有验证每个仓库的每个文件许可。参考不构成复制授权；复用前必须检查对应文件及依赖许可，并保留必要 notice。未知时独立实现。
