# 验收用例清单

M0 本轮结果见逐项状态；R14 仅完成初始性能基线。其余条目仍为 NOT_RUN，不能视为已实现。

## R00 锁定 Rust 基线、契约和可重复验收
- [x] R00-A01 — 所有原有原生测试、fmt、clippy 和现有三个检索门禁实际执行并记录退出码；受限环境明确 not_run。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/m0-final/results.json。
- [x] R00-A02 — 以同一 JSON fixture 调 CLI/MCP 后可解释字段差异；八个工具及两个 CLI alias/一个 MCP alias 仍可用。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/m0-final/results.json。
- [x] R00-A03 — 测试不新增、修改或读取测试范围外的真实记忆内容；原生运行测试清空 PATH 仍可工作。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/m0-final/results.json。
- [x] R00-A04 — 新文档的默认缓存名是 rust-index.sqlite / vectors-rust.sqlite；不宣称 Python runtime/API 或 Hermes provider 仍受支持。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/m0-final/results.json。
- [x] R00-A05 — 打包清单没有 .mnemosyne、credentials、models、target、私有报告；源包解包后能离线使用已缓存依赖构建，缺缓存时说明限制。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/m0-final/results.json。
- [x] R00-A06 — 内置 50-query 检索门禁和 2-case LongMemEval 样本分别标识，未声称跑了完整公开基准。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/m0-final/results.json。

## R01 补来源保留与生命周期语义回归
- [x] R01-A01 — 同 source 精确重放只有一个有效记录；仅 source 改变时第二来源不丢失。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r01-regression.txt; evidence/m0-final/results.json。
- [x] R01-A02 — 同标题的 port 8080 与 healthcheck /health 共存；改成 port 8081 仍保留新事实。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r01-regression.txt; evidence/m0-final/results.json。
- [x] R01-A03 — 长正文只变一个版本号/否定词不被判 duplicate；evidence 或 expires 更新仍能保存。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r01-regression.txt; evidence/m0-final/results.json。
- [x] R01-A04 — 使用低 archive 阈值保留在 working 的 superseded 记录，多次 maintain 后仍为 superseded 且默认搜索不可见。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r01-regression.txt; evidence/m0-final/results.json。
- [x] R01-A05 — 归档后的 superseded 记录在 --archive 且不含 include-superseded 时不会因为状态改写重新出现。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r01-regression.txt; evidence/m0-final/results.json。
- [x] R01-A06 — 高 strength、高 access_count 的 superseded 记录不成为 core candidate；旧 valid 记录维护行为没有意外变化。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r01-regression.txt; evidence/m0-final/results.json。

## R02 向量缓存自愈与推理快照一致性
- [x] R02-A01 — 把一条原本新鲜缓存改为零向量、错误维数或畸形 JSON 后，下次 backfill 修复它，而不是永远跳过。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r02-regression.txt; evidence/m0-final/results.json。
- [x] R02-A02 — 正常同输入、同模型的重复 backfill 不调用 provider；只改变 access_count 不重算。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r02-regression.txt; evidence/m0-final/results.json。
- [x] R02-A03 — 推理期间正文、dimensions、model/revision 改变时旧结果不落为新鲜向量。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r02-regression.txt; evidence/m0-final/results.json。
- [x] R02-A04 — 查询推理期间 profile 改变时旧缓存与新查询向量不混用；词法结果仍返回且有降级诊断。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r02-regression.txt; evidence/m0-final/results.json。
- [x] R02-A05 — 断开 ONNX/HTTP 或损坏向量 SQLite 时，未启用模型的基本读写/MCP 不受影响。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r02-regression.txt; evidence/m0-final/results.json。
- [x] R02-A06 — provider 返回乱序批次、重复 index、NaN、Inf、零向量、错误维数均有明确处理；真实模型验收与 mock 分开。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r02-regression.txt; evidence/m0-final/results.json。

## R03 MCP 原生协议与有界输入验收
- [x] R03-A01 — 正确初始化→工具列表→调用→断开能在 stdio 和已有 SSE 运行，CLI aliases 不受影响。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r03-regression.txt; evidence/m0-final/results.json。
- [x] R03-A02 — 无 id 通知不产生伪造 id:null 响应；非法请求不写 store；具体允许的通知记录在契约。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r03-regression.txt; evidence/m0-final/results.json。
- [x] R03-A03 — 工具业务失败以所声明协议的工具错误结果返回，parse/params 错误仍是 JSON-RPC 错误。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r03-regression.txt; evidence/m0-final/results.json。
- [x] R03-A04 — 超大/非法 UTF-8/负 limit/错误类型/未知字段均不导致进程 panic、越界写或无界缓冲。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r03-regression.txt; evidence/m0-final/results.json。
- [x] R03-A05 — 同一服务器在 cwd=/ 交替读写两个显式项目，不串库；关闭 global exposure 后每个相关工具都遵守。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r03-regression.txt; evidence/m0-final/results.json。
- [x] R03-A06 — 没有模型库、Python、外网和 HOME 中旧包时，八个基础工具仍满足其非模型契约。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r03-regression.txt; evidence/m0-final/results.json。

## R04 稳定身份、来源事件和写入结果 v2
- [x] R04-A01 — 两 agent 转述同一源事件，不增加独立证据；新独立事件支持相同事实时证据不丢。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json。
- [x] R04-A02 — 同一源事件产生两条不同 findings，都能保存；并发重放每条仅提交一次。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json。
- [x] R04-A03 — 同幂等键不同 payload 明确 IDENTITY_CONFLICT；重试不静默更换结果。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json。
- [x] R04-A04 — 未知 flat frontmatter、中文、引号、逗号、反斜线 round-trip；legacy 文件不被一次读操作全量改写。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json。
- [x] R04-A05 — agent 自填 verified 不获可信验证等级；来源缺失输出 unknown，不伪造 user_asserted。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json。
- [x] R04-A06 — 具有同裸 ID 的两个 store 可通过新 MemoryRef 正确定位；旧入口行为记录且可测试。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json。
- [x] R04-A07 — 新旧 JSON、MCP v1/v2 契约可区分；真实配置不变，schema 首次升级可预览和在副本上回滚。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r04-regression.txt; evidence/m1-final/results.json; evidence/r05-regression.txt。

## R05 ContextBundle 增量完善：路径、修订与压缩周期
- [x] R05-A01 — core+标题+来源+警告+提示+检查点占位全部纳入预算；mandatory 超限沿 fail-safe 输出空 context+stderr。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r05-regression.txt; evidence/m1-final/results.json。
- [x] R05-A02 — 同一 revision 同 epoch 不重复注入；同 ID 内容更正或 context_epoch 改变可以再次注入。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r05-regression.txt; evidence/m1-final/results.json。
- [x] R05-A03 — 只改访问计数不触发重新注入；另一 host、另一 project、另一 channel 不共享错误抑制状态。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r05-regression.txt; evidence/m1-final/results.json。
- [x] R05-A04 — backend/config.rs 与 frontend/config.rs 有区别；找不到完整路径时有可解释 basename fallback。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r05-regression.txt; evidence/m1-final/results.json。
- [x] R05-A05 — 缓存/会话锁争用时仍交付可用有界上下文；被裁掉的条目不记 injected/access。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r05-regression.txt; evidence/m1-final/results.json。
- [x] R05-A06 — CLI、MCP 和 hook 在相同输入下候选与预算一致，通道差别仅在合法外壳/读取提示。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r05-regression.txt; evidence/m1-final/results.json。

## R06 原生 TaskCheckpoint：真正接续工作
- [x] R06-A01 — A 修改两文件、执行测试且 2 fail，B 接手必须看到未解决的 2 fail，不显示任务已验证完成。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r06-regression.txt; evidence/m1-final/results.json。
- [x] R06-A02 — 只有“准备跑测试”的文字不变成 executed/passed；reported evidence 不伪装成受信观测。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r06-regression.txt; evidence/m1-final/results.json。
- [x] R06-A03 — 同 commit 但 dirty 文件变化后，测试证据标记需复验。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r06-regression.txt; evidence/m1-final/results.json。
- [x] R06-A04 — 两个 agent 并发 update：一个成功，另一个 CAS conflict；不采用 last-writer-wins。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r06-regression.txt; evidence/m1-final/results.json。
- [x] R06-A05 — 关闭/到期 checkpoint 不再默认注入，但显式查看可保留记录；不会变成永久 pitfall。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r06-regression.txt; evidence/m1-final/results.json。
- [x] R06-A06 — 四宿主 12 个有方向的跨宿主组合使用协议 fixtures 验证身份与交接；与真实模型会话统计分列。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r06-regression.txt; evidence/m1-final/results.json。

## R07 语义修订与可恢复写入的共同基础
- [x] R07-A01 — 新建、更正、supersede、关系变化可重建历史；只访问或调热度不产生新 semantic_rev。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。
- [x] R07-A02 — 在每个持久提交阶段注入失败，恢复后得到已提交一致状态或明确冲突，没有半数事实被隐藏。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。
- [x] R07-A03 — 保留既有跨 store 五个中断阶段测试；新单 store 多对象操作不破坏它们。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。
- [x] R07-A04 — 恢复期间用户手改任意目标时不覆盖；journal 与现场保留可诊断。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。
- [x] R07-A05 — v1/v2 旧 pending journal 可恢复；新 journal 中越界/符号链接/重复目标/哈希不符被拒绝。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。
- [x] R07-A06 — 旧记忆首次纳入历史只记录当前快照与 coverage 起点，不能生成虚构的过去 body。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。
- [x] R07-A07 — 从规范 Markdown+manifest/证据重建 SQLite 后，当前状态与历史版本不丢失。
  状态：PASS；命令：见 EXECUTION_LOG.md 与证据文件；退出码：0；证据：evidence/r07-regression.txt; evidence/r07-final/results.json。

## R08 system-time 历史查询与保守代码适用性
- [x] R08-A01 — 在 T1 保存 PostgreSQL，T2 明确替代为 SQLite；T1≤T<T2 只看到当时版本。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final`；证据：`evidence/r08-regression.txt`、`evidence/r08-final/results.json`。
- [x] R08-A02 — 当前已经过期的事实在过去有效时间仍可查；跨日期/偏移边界规则一致且输出解释。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final`；证据：`evidence/r08-regression.txt`、`evidence/r08-final/results.json`。
- [x] R08-A03 — 没有历史覆盖时返回 partial/unknown；不将 recorded_at=今天的旧文件伪装成其 created 那天的完整快照。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final`；证据：`evidence/r08-regression.txt`、`evidence/r08-final/results.json`。
- [x] R08-A04 — 加入未来文档、当前 supersedes 边、当前向量后，不会泄漏未来正文/关系进历史结果。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final`；证据：`evidence/r08-regression.txt`、`evidence/r08-final/results.json`。
- [x] R08-A05 — 历史只读查询不改 usage、expiry、session 去重或 canonical memory。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final`；证据：`evidence/r08-regression.txt`、`evidence/r08-final/results.json`。
- [x] R08-A06 — 切到旧 commit、相同 commit 的脏工作区、两分支同名文件：适用性不同或 unknown，不自动套用最新结论。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final`；证据：`evidence/r08-regression.txt`、`evidence/r08-final/results.json`。

## R09 Reconciliation 与 Proposal Ledger
- [x] R09-A01 — 相同主题互补事实为 CREATE/REFINE 候选，不自动失效；多值属性不产生假冲突。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R09-A02 — 同属性不同环境被识别为不同上下文；同范围单值冲突有证据说明但不自动选择赢家。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R09-A03 — 不同来源同事实 SUPPORT 追加来源；同源转述不倍增证据。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R09-A04 — 提案生成后目标被另一个 agent 修改，批准返回 stale 且无半应用。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R09-A05 — 重复批准/applied 重放幂等；对不具审批授权的 MCP 调用拒绝，其他工具正常。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R09-A06 — 语义修改全部经过 R07 恢复/历史链；undo 不覆盖后续外部修改或删除证据。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。

## R10 可重复维护与热度/有效性分离
- [x] R10-A01 — 同一天运行一次/十次维护结果相同（per_day），per_run 保留旧契约。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R10-A02 — 启用 per_day 的旧库不因历史年龄被一次归档；时钟回退、跨日边界可重复测试。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R10-A03 — 被频繁注入只提升热度，不提升 verification 或解除 superseded。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R10-A04 — pinned 关键低频知识不因热度归档，但明确 supersede/expiry 仍按公开策略处理。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R10-A05 — dry-run 不改文件/历史；实际状态变化只有一份对应 revision。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。

## R11 有界 sleep 与宿主辅助提案
- [x] R11-A01 — 关闭所有模型仍能执行规则 sleep/report；不发生网络请求。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R11-A02 — 相同 input snapshot 重跑不重复生成等价提案；不同 revision 会生成新候选或 stale。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R11-A03 — 模型失败、非法输出、半批提案失败时 cursor 不丢未处理输入。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R11-A04 — 模型声称高置信度 caused_by/supersedes 仍待审，不直接改 Markdown。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R11-A05 — 输入/输出/提案数超上限时明确 partial 与下一游标，不无限循环。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R11-A06 — 报告含来源但不含凭据；未授权 sleep 不能改 core、执行命令或启用 cron。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。

## R12 可追溯的派生知识页
- [x] R12-A01 — 每条关键结论可追溯到具体 source revision；不存在的引用使生成失败或明确 partial。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R12-A02 — 源被替代后视图不再作为当前确定知识注入；旧视图仍可审计。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R12-A03 — 原始事实与派生摘要不构成两个独立证据；相同上下文不双重占预算。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R12-A04 — 人工 core 不被自动改；视图外部手改触发冲突而非静默覆盖。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R12-A05 — 无模型情况下仍能按模板生成索引式视图，不伪称完成语义综合。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。

## R13 原生快照、迁移与安全恢复
- [x] R13-A01 — 备份→空目录恢复→重建 SQLite 后 facts/provenance/history/checkpoints/proposals 数量与哈希符合 manifest。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R13-A02 — 路径 ../、绝对路径、符号/硬链接、重复条目、超大解压、损坏哈希均被拒绝。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R13-A03 — 恢复失败不覆盖原 store；真实用户数据不作为默认测试输入。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R13-A04 — 存在未完成跨 store 操作时拒绝不完整迁移，不盲删 coordinator decision。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R13-A05 — 默认包中没有凭据/API selectors/敏感 transcript；脱敏策略及被排除项在 manifest 可见。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R13-A06 — restore 保身份与 fork 新身份行为各自明确，旧裸 ID 引用不会静默指向另一独立库。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。

## R14 实测驱动的性能与检索评测
- [x] R14-A01 — 发布可复跑命令、原始 CSV/JSON、数据哈希和环境，所有性能数值与明确提交绑定。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R14-A02 — 旧三个 recall 门禁不降低；benchmark fixture 不作为 training/tuning 后唯一测试集。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R14-A03 — 词法→向量→图→rerank 消融明确每一路实际启用状态，不混用 mock 和真实推理结论。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R14-A04 — 外部同大小正文修改、删除、rename、并发读写、缓存损坏、pending recovery 在优化前后行为一致。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R14-A05 — 跨 agent 端到端使用无记忆/本版/改进版相同任务与预算，记录错误事实使用率和接手续作结果。
  状态：PASS（有界合成任务）；命令：`python3 tests/native_agent_eval.py target/debug/mnemosyne --old-binary target/baseline-v1/debug/mnemosyne --codex "$(command -v codex)" --auth-file "$CODEX_HOME/auth.json" --output docs/plans/native-evolution/evidence/r14-agent-eval.json`；证据：`evidence/r14-agent-eval.json`。固定 Luna max，三个独立真实 CLI 消费者；生产者为 fixture，不代表真实四宿主会话或广泛任务成功率。
- [x] R14-A06 — 索引冷建变慢与热查询收益分别报告，不用某一项平均值笼统宣称全面提速。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。

## R15 发布候选收口与执行交接
- [x] R15-A01 — 全部任务验收条目有 pass/fail/not_run/blocked；没有未运行却写 pass 的条目。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R15-A02 — 源码打包白名单继续有效；二进制不依赖 Python 内核，开发 smoke 脚本可以保留。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R15-A03 — 从旧 1.0.0 副本升级→复跑→快照恢复完整演练；所有 pending 操作有明确处置。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R15-A04 — 四当前宿主的协议合同通过，真实宿主/真实模型证据单列；不重新宣称 Hermes support。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
- [x] R15-A05 — 版本与 schema 变更、回滚限制、尚未实现选项在中英文文档一致；未经授权没有发布或安装操作。
  状态：PASS；命令：`python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne`；证据：`evidence/r09-r15-regression.txt`、`evidence/final/results.json`。
