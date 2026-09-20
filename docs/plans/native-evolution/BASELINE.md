# 基线审阅记录（非本地测试报告）

- 检查日期：2026-09-19
- 仓库：Furinelle/Mnemosyne，master
- 固定 commit：`d05f02853f42771ae9ca04d78b79739d01f1a76c`
- package：1.0.0，Rust edition 2024，MSRV 1.95
- 托管 CI：run [35441992545](https://github.com/Furinelle/Mnemosyne/actions/runs/35441992545)，核查时 completed/success。
- 本轮本地 Rust 编译/测试：NOT_RUN（未找到 Cargo；clone DNS 失败）。
- 这次没有修改远端仓库、真实 store 或宿主配置。

## 已确定的源码事实

| ID | 事实 | 证据位置 |
|---|---|---|
| F01 | write_entry/duplicate_entry 已撤掉相似度 supersede，但精确比较没有 source | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs |
| F02 | maintain 可在低 strength 分支把 status 赋为 deprecated；需测试 superseded 不被改写/晋升 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs |
| F03 | assemble 已覆盖 base/entries/suffix 的 estimated 预算；必须内容超限明确错误 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs |
| F04 | session 已有 host/channel/session 隔离；memory identity 为 scope/path/id，未含语义 revision/epoch；file_touch 仍 basename | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs |
| F05 | vectors 已有输入哈希/model fingerprint/dimension 与写回 CAS | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/vectors.rs |
| F06 | cached_vectors 验坏向量；cached_hashes 不验证 vector_json，用于 backfill fresh 判定 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/vectors.rs |
| F07 | fingerprint 本地模型/词表按字节哈希；查询 lane 先 fingerprint 再 embed，需验证计算中配置变化的快照一致性 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/models.rs / src/vectors.rs |
| F08 | 关系 journal 有 before/after hash、路径检查、coordinator 与多阶段恢复；成功后清 journal，不是语义历史 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/relations.rs |
| F09 | MCP business error 目前走 -32603；通知特殊处理/声明协议/stdio 无显式大小限制需按 spec 验证；SSE 已有 1MiB 限制 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/mcp.rs |
| F10 | 当前原生 CLI 没有 checkpoint/history/proposals/sleep；迁移文档明确这些是后续迭代 | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs / docs/rust-migration.md |
| F11 | interface.md 仍引用 Python API 与旧 index.sqlite | https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/interface.md |

## 证据等级

F01/F02/F06 描述具体静态路径；它们是应先写回归验证的缺口，不是本轮已经运行得到的失败。F07 为一致性风险检查点；不要直接宣称在所有模型后端可复现。其他为已读取文件/接口中的实现事实，不能据此断言所有可能输入都正确。

docs/rust-local-cutover.md 记录某些阶段的本机模型、hooks/协议、44 个 Rust 测试及性能对照。它自己区分了证据快照；本计划不把它转换成 d05f028 所有真实宿主已跑的声明。CI success 是另一个独立可核查事实。

## 取消或替换旧计划

- Rust probe / Python→Rust：取消，已原生重构。
- Jaccard 自动 supersede 止损：主体已有；改为 F01/F02 小缺口回归。
- 整包 Context Compiler v1：已有；增量做结构化结果/路径/epoch/修订。
- model/dimension/hash/backfill CAS 从零实现：已有；增量做坏行修复/诊断/查询快照。
- graph hopping：已有，做 ablation。
- 两文件/跨 store relation recovery：已有，保留并复用。
- 保持 Python import/Hermes provider：取消，当前 native migration 已明确退役。
- provenance/history/checkpoint/proposals/sleep/views：继续，但 Rust 路径与依赖重新设计。
