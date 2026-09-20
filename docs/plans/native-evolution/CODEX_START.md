# 交给 Codex / Astra 的启动提示

请在 Furinelle/Mnemosyne 工作区实施 `docs/plans/native-evolution/PLAN.md`。
这是 Rust 原生演进计划；参考基线 `d05f02853f42771ae9ca04d78b79739d01f1a76c`，package 1.0.0。

先阅读当前 AGENTS.md、BASELINE.md、PLAN.md、BACKLOG.json 和 ACCEPTANCE.md，再核对实际 HEAD、未提交修改、Cargo.toml/rust-toolchain.toml、原有 tests 与 CI。HEAD 已推进时按实际代码做差异映射，不能 reset 用户工作区或重新实现已经修复的功能。

默认第一批执行 R00→R01→R02→R03，并建立 R14 的性能基线。每个任务完成新增回归、实现、测试、diff review 和执行记录后再进入下一个。后续按 depends_on 实施 R04→R05→R06，优先交付 provenance、上下文增量和 TaskCheckpoint；再做历史、提案、sleep、视图和快照。不可把所有任务揉成一次大重写。

必须保留 Rust-only 内核、Markdown 知识原文、派生 SQLite、现有 CLI/MCP/四事件和原生 aliases。Python import API 与 Hermes provider 已退役，不能为了旧计划重新恢复。开发用 Python stdlib smoke 可保留，不是运行时依赖。

先验证而非假设以下已完成能力：保守精确写入、整包 estimated budget、向量 input hash/model fingerprint/dimension/回填 CAS、跨 store 关系恢复。不要重复开发它们。先用 Rust 回归核查 source 在 duplicate 中遗漏、superseded 生命周期、坏向量不能自愈、MCP 错误/通知边界。

所有运行测试使用临时 HOME/MNEMOSYNE_HOME/项目目录，保留正确工具链定位。不能读写真实记忆或改宿主设置；不运行生产模型 endpoint；没有资产就报告 NOT_RUN；不自动下载模型。不得 push/tag/publish/install 或在真实 store 应用 proposal/restore。

模型计算在锁外；写回检查 snapshot；语义修改走可恢复 mutation；旧 journal 保持兼容。不要将完整 YAML 或通用分布式事务引擎带入本轮。未知来源不伪造、重复转述不增信、same fact_key 不自动 supersede、普通 agent 不自动审批自己的破坏性提案。

每个验收条目记录：命令、退出码、pass/fail/not_run/blocked、证据路径。当前计划没有本地执行 Rust 测试，托管 CI success 也不是你修改后的通过证明。不得降低 recall 门槛、删反例、把 fallback 冒充目标 pipeline，或把协议 fixture 宣称为真实应用会话。

新增命令/字段先更新 versioned contract，再实现 CLI/MCP 共用逻辑。发现计划与现有实现冲突时，用最小可运行方案修正计划和日志，并解释证据；不要把做不完的任务标成已完成。

当前批次结束时输出：HEAD/diff、完成任务与验收编号、实际测试结果、未解决问题、迁移/回滚影响和下一个依赖已满足的任务。上下文不足时写入 EXECUTION_LOG.md；不承诺后台继续。
