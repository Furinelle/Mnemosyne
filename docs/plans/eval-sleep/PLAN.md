# Mnemosyne v2.0.1 后续工作：验收可信化与剩余边界

审阅日期：2026-09-26。仓库：Furinelle/Mnemosyne。
冻结基线：`a0da375a872001b57cb4f259492961de6edc2de6`；Cargo version：2.0.1。
最近运行时代码提交：`21611f9528164b4db4594427f6cb6d2856942b9b`；a0da375 为证据/文档提交。
CI：run 35515845048，读取时 success。执行本计划前重新核实实际 HEAD。

本文是下一批待实现任务，不是完成报告。参考源码固定链接见 SOURCES.md。
本轮独立执行仅限附带的 Python 评分表达式最小反例；未编译 Rust、未重新执行 159 项项目测试、未运行真实模型或宿主、未修改远端仓库。

## 1. 先承认已经完成的修复

最新代码与测试记录已覆盖以下变化。后续以复跑回归确认，不重复实现：

- ingest/distill/session_end 在升级库中通过 WriteRequestV2 进入原有 provenance 写入；预览不写库，显式旧 writer 仍拒绝。
- reconcile 先检查 environment/value，再判断相同正文。上轮“同正文不同单值判 SUPPORT”分支已修正。
- 正式修订后追加 SUPPORT 检查已记录历史，来源支持的 semantic revision 单独绑定；旧来源绑定未知时保留 null，不反向猜测。
- sleep 已改为清点元数据、只读取当前有界页；上轮全库超过 4 MiB 即不能读取第一页的问题不应作为未修项重报。
- 已有分阶段计时，模型指纹与初始化也有单独记录。计时是 inclusive、可重叠，不能简单相加当端到端总时长。

159 tests / 15 local gates / 真实缓存 ONNX 是仓库作者对指定构建的执行记录；本轮没有独立复跑。当前 CI success 只证明其实际配置中的门禁，不能代替离线评分测试、完整跨宿主实验或所有原验收语义。

仍公开标 PARTIAL 的主要条目是 R11-A05（部分超限续页能力）及 R14-A05（完整跨宿主/模型端到端证据）。不要把记录准确性的改进重新变成“全部通过”。

## 2. 本轮新增发现及范围

### 2.1 评分脚本：执行成功和回答正确混在一起

`tests/native_agent_eval.py` 计算 facts_correct/handoff_correct，却使用“没有 tool_items”决定 status=pass；报告 complete 仅看三个 arm 的 status。candidate_handoff 的 unsupported_answers 直接为 0，未逐字段核验。

本包 `scorer_counterexample.py` 忠实摘取相关评分表达式，输入三个错误字段，得到 candidate facts_correct=0、handoff_correct=false，但 status=pass、unsupported_answers=0、report complete=true。它没有调用原项目 harness、真实 agent 或模型，只验证上述表达式。

这不证明历史模型回答实际上错误，而是证明当前 pass/complete 不能作为回答质量门禁。先修评分，再花模型额度扩大实验。

### 2.2 sleep：已修输入页预算，但输出/规模边界仍未收口

当前 `src/sleep.rs` 仍有 MAX_INPUTS=10000、MAX_RECORD_BYTES=128 KiB、MAX_REPORT_BYTES=64 KiB、MAX_PROPOSALS=20 等保护。finish 在输出超限时直接拒绝，尚无输出页的持久继续协议。inventory_snapshot 仍检查全体清单，清单内单个超大文件会阻断整轮 export。

这些上限本身不是漏洞；下一步是区分“可分页的批次额度”和“必须保留的安全硬限制”，明确 partial/blocked/next_cursor，而不是删除上限。不能要求任何大小的非法单条记录都被完整接收。

### 2.3 性能基准：计时已有，但默认语料仍是 legacy 存储

`tests/native_perf.py` init 后直接写 working/*.md，未升级 store、没有来源 sidecar 或语义历史。它能量化基础检索，却不能代表用户启用 provenance/history/checkpoint 后的日常成本。

下一步要增加真实有效的 V2 合成库基线，再按测量决定是否优化；不重复做 Rust 化，也不先假定某个模块一定最慢。

## 3. 执行顺序与完成定义

默认顺序：NX00 → NX01 → NX02。NX03 的基线采集可与 NX02 在隔离目录并行；NX04 依赖 NX01 评分器，真实运行只在明确允许模型调用、对应宿主可用时开展。

代码与离线回归可以直接实施，不需要每项重复征求确认。真实模型、真实宿主配置、真实库迁移/恢复、安装、push/tag/release 均不由本计划自动授权。缺少资产或权限时准确标 NOT_RUN/BLOCKED，并继续可独立完成的工作。

完成分三层：implemented（代码存在）、offline_verified（离线行为验证）、live_verified（指定环境真实执行）。不要用任意一层替代另外两层。

## NX00 — 基线与已修问题回归（入口门禁，不做第二轮重构）

落点：现有 tests/native_only.rs、tests/provenance_v2.rs、tests/reconciliation.rs、tests/sleep.rs、tests/native_acceptance.py、.github/workflows/ci.yml。

1. 读取实际 AGENTS.md、HEAD、dirty diff、Cargo toolchain/features 与当前 CI；不 reset、不覆盖用户文件。HEAD 变化时先检查下列问题是否已修。
2. 在临时 HOME/MNEMOSYNE_HOME/项目库内复跑：升级后 ingest、heuristic/host distill 和真实 Stop 命令的正向写入、重放、只读预览；验证实际新增的正文、来源事件和历史，而不是只检查 fail-safe 的 exit 0。
3. 回归相同正文不同结构化单值应 CONTRADICT、不同环境应 CONTEXTUALIZE、多值并存、相同值来源重放与 SUPPORT。
4. 回归 create→revise→SUPPORT→历史 provenance；旧来源不可被改绑新修订，外部未记录编辑不能借 SUPPORT 变成可信历史。保留当前 old/new writer 兼容用例。
5. 回归 45×100000 字节合成记忆的 sleep 输入续页；每页有限，重放/导入待审提案不导致无故丢页。

验收：
- NX00-A01：记录 HEAD、源文件摘要、binary hash、toolchain、实际命令和退出码；作者旧日志与本次执行分列。
- NX00-A02：升级库的正向写入与重放通过，读/预览不写，旧 writer 保护仍在。
- NX00-A03：结构化值/环境/多值四类对照正确，旧事实无未经授权失效。
- NX00-A04：修订前后来源绑定和 historical prefix 正确，旧 unknown 保留 unknown。
- NX00-A05：原有 default/all-features、fmt/clippy、三个检索门禁不降低；环境阻塞不伪装 PASS。

## NX01 — 修正评测器的质量判分（P0，第一个修改提交）

落点：`tests/native_agent_eval.py`；新增独立纯评分模块/测试（例如 `tests/native_eval_scoring.py`、`tests/native_eval_scoring_test.py`）；`.github/workflows/ci.yml`。

实施边界：

- 分离 invocation_status、protocol_status、quality_status。退出码 0 和无非法工具调用只是执行条件，不是质量成功。
- 将 oracle 明确到 task×arm×field：正确值、允许的规范化、应回答 unknown 的字段、允许的证据集合。支持判断基于该组实际交付的上下文，不是把全局正确答案泄漏给模型。
- 每个 arm 都计算事实正确、无依据断言、错误拒答和 next_action 恢复。candidate 不能默认 unsupported=0；baseline 数据库/端口答错同样计入。
- 将模型 JSON 验证和评分提取为无网络纯函数。不要用包含 unknown 子串就判拒答：如“unknown, but database is Oracle”仍包含断言。允许哪些等价拒答表达需明确。
- complete 如保留，限定为执行完整性并更名/新增独立 acceptance_passed；质量通过必须满足该组 oracle、零不支持的断言、合法协议，且必需组都完成。报告升级 schema，避免旧消费者误解字段。
- 超时、模型不存在、认证不可用、输出损坏不能标质量 PASS。运行质量失败用非零进程退出码；仅缺少可选环境可以 NOT_RUN，但不能 overall_pass=true。
- 保存原始旧证据，不篡改历史回答。已有回答允许在不重调模型的情况下用新 scorer 重新判分，但报告必须写“旧回答离线重评分”，不是新一轮 E2E。
- 将无凭据/无网络评分单元测试接入 Linux/macOS CI；不要把真实模型调用接入默认 CI。

验收：
- NX01-A01：候选组三个字段全错、没有工具调用时 quality FAIL、acceptance_passed=false。
- NX01-A02：候选仅 next_action 错也不通过，不能被 facts_correct=2 掩盖。
- NX01-A03：without_memory 准确 unknown 可通过拒答要求，猜中隐藏答案也不算有证据回答。
- NX01-A04：v1_retrieval 的 database/port 错误计入；没有提供 next_action 时编造操作被识别。
- NX01-A05：对照正确答案全部通过；与规范化契约不等价的回答不会被模糊字符串匹配放过。
- NX01-A06：混有 unknown 与肯定断言、非法 JSON、少字段、超预算/非法工具调用都有独立失败用例。
- NX01-A07：超时/模型不可用/认证失败是执行错误或 NOT_RUN，不是正确性 PASS。
- NX01-A08：未执行的 arm/缺失结果不能使总验收通过；报告明确 completed_arms/required_arms。
- NX01-A09：评分回归在默认 CI 实际运行，不要求凭据，不调用模型；保留本包最小反例作为负对照。

## NX02 — sleep 输入/输出继续协议（P1，补 R11-A05）

落点：`src/sleep.rs`、`src/proposals.rs`、`src/main.rs`、`src/mcp.rs`/`src/mcp_tools.json`（仅现有 sleep 入口需要时）、`tests/sleep.rs`、接口文档。

先写小 ADR，将输入位置、模型/host 输出位置和报告提交位置分开。不要继续用一个 next_cursor 同时表示“读完了”和“所有结果已持久化”。

建议的兼容实现：旧 Batch v1 继续可读；新增带 run_id/snapshot_ref/input_cursor/output_cursor 的版本化小记录。字段名称可调整，但持久化语义必须一致。

- 输入每次受记录数和累计字节预算约束。保持已修的 4 MiB 页预算，不重新全库读取正文。
- 在声明支持的库规模内，为目录枚举/清单建立有界继续方式，避免“页大小=1 仍先完整读取全部正文”。区分每请求数量额度和整个库的硬规模上限；超过后者允许明确 BLOCKED，不无限扫描。
- 单条超过安全上限允许拒绝/记录 blocked reference，默认不截断成另一条事实。允许其他合法记录继续，但整轮仍 partial，且留下待处理项；不能默默丢弃并报 done。
- 多于20个提案或超过报告页预算时，host/客户端分批提交输出；每个请求仍服从已有传输体积上限。不承诺接收超大单次 JSON 再自动切片。
- 输出 chunk 使用确定性幂等 ID 和收据。只有所有声明的输出 chunk 验证并持久化、报告完成提交后，才推进已处理输入游标。提案发布不自动批准。
- 复用原有原子写、MutationPlan/恢复/前置条件和路径白名单；不要创建第二套通用事务引擎。操作日志落盘前后故障能重放，未知外部修改保留现场。
- 检查目标 revision/hash，不允许过期输出套到新事实。游标过期给出明确恢复路径，不伪装成功。普通访问热度是否影响快照要写成显式契约并测试。
- 一轮输出的全部记录/blocked/partial 状态可汇总验证，保证没有重复应用、没有未报告遗漏、没有无限返回同一游标。

验收：
- NX02-A01：45×100000 字节语料分页完成，无重复遗漏；原有重放回归保持。
- NX02-A02：25个以上合法提案分多次提交完成，每次不超现有上限，均为 pending。
- NX02-A03：总报告超过64 KiB时分片保存/返回；输入游标不提前标完成。
- NX02-A04：在清单、提案页、报告、游标的各持久阶段注入失败后，可重试且不重复应用。
- NX02-A05：并发两个客户端提交同一输出页，幂等或明确冲突；不能互相覆盖游标。
- NX02-A06：单条超大/损坏记录仍拒绝，其他合法记录可继续；汇总 partial/blocked，原文和 core 不变。
- NX02-A07：快照源发生语义更改后拒绝旧结果；访问热度与显式输入修改按已公开契约处理。
- NX02-A08：越界路径、symlink/hardlink、伪造游标、非法序号仍被拒绝，不把安全限制删掉。
- NX02-A09：终页为空、limit极小、输出零提案、超过声明库规模均有确定终止行为与状态。
- NX02-A10：R11-A05 只有在相应原要求覆盖后才升级 PASS；保留的硬限制或范围偏差在 ADR/ACCEPTANCE 明示，不自行认定例外等于原要求全满足。

## NX03 — 用真实 V2 合成库补性能基线（P1；优化另列P2）

落点：`tests/native_perf.py`、已有 timing 模块（实施时确认名称）、`src/search.rs`、`src/provenance.rs`、`src/revisions.rs`、`src/context.rs`、模型相关模块。

- 保留 legacy 场景作对照，新增通过正式 store-upgrade 与 V2 API 创建的有效库。文件名/ID/manifest/sidecar/history 必须一致，不复用 legacy 直接写文件的方式冒充 V2。
- 控制实验规模：100条含较深历史、1000条含多个revision/来源、10000条浅历史；记录实际计数与磁盘预算，不机械跑所有笛卡尔积。
- 覆盖冷索引、热搜索、单文件更改、Stop正向写入、prep+checkpoint、PreToolUse、pending恢复独立用例。明确进程启动、热/冷磁盘缓存和模型session状态；源码commit与binary hash都固定。
- 当前分阶段计时已经存在，直接复用。duration 为 inclusive/overlap 时禁止相加；做父子span/exclusive分析需明确测量方法。
- 进行固定预热、至少5次基线采样；相对性能门槛在取得稳定同机数据后提出，不使用5点p95当高可信尾延迟，也不和旧inject/新PreToolUse混算收益。
- 真模型资源未提供时标 NOT_RUN；有显式授权的已缓存 ONNX 时单独测，不下载、不启动云API。
- 先提交原始测量和瓶颈判断。确有热点时才以独立补丁改善重复枚举、重复读取或同请求内缓存；不按路径/mtime代替当前内容校验，不弱化external-edit与pending recovery保护。

验收：
- NX03-A01：报告分别列legacy/V2库schema、记忆/来源/修订/检查点数量，基准确实走正式V2路径。
- NX03-A02：每组报告command/commit/binary hash/corpus hash/环境/raw samples/实际lane，非目标fallback明确标出。
- NX03-A03：正向Stop确实产生新V2事实，prep确实包含完整checkpoint next_action；空输出不能当快速成功。
- NX03-A04：包含历史与证据的100/1000/10000范围语料完成或准确BLOCKED，不以小样本替代大库声明。
- NX03-A05：独立记录无模型和已授权ONNX场景，inclusive计时不双重计算。
- NX03-A06：任何优化前后，外部编辑、同大小替换、rename/删除、并发、坏缓存、未决恢复结果一致。
- NX03-A07：性能回退按同条件对照披露；不要求为达速度目标删除安全检查或改变语义。

## NX04 — 生产者到消费者的跨宿主验证（P1，依赖 NX01）

落点：`tests/native_agent_eval.py`、新增有界host runner/场景清单、已有native_only/checkpoint fixtures、验收记录。

现有12个方向的协议fixture继续保留。新任务不是将fixture重命名为真实测试，而是增加独立运行、退出并读取共享记忆的生产者与消费者。

- 先实现没有账户也能跑的runner契约与模拟失败测试。实际宿主命令、模型、reasoning、最大调用数/预算、目录均由显式配置提供；不硬编码某个历史模型别名，也不默认回退到另一个模型。
- 最小真实纵向链：生产者处理合成小项目，实际遇到失败/完成部分操作，通过原生write/Stop保存来源与checkpoint；退出；消费者用独立新会话仅靠自身项目和Mnemosyne读取接续，不获得生产者对话原文或隐藏答案。
- 首先验证Codex↔Claude两个方向；其余宿主仅在环境支持且另行授权时扩展。四宿主完整覆盖的目标仍为12有向组合；未提供宿主的行标BLOCKED/NOT_RUN，不能复制另一个模型结果补齐。
- 场景至少包括未完成测试、事实修订、不同环境、脏工作区使旧证据过期、证据不足应unknown。运行范围和统计置信限制明确。
- no-memory / 固定baseline / 当前版使用相同任务、固定初始代码快照、同模型或明确模型分层、相同上下文上限。生产者完成后保存受控状态，用只读副本构造各arm，避免条件间污染。
- 用NX01 scorer逐字段判分，并记录运行成功率、续作正确率、无依据断言、错误拒答、输入/输出成本和时延。无记忆不是要求模型乱答，而是期待在证据不足时正确拒答。
- 成本/账户访问属于显式opt-in；只用临时HOME/CODEX_HOME/MNEMOSYNE_HOME和合成数据。凭据不进日志/压缩包/命令行示例；只保存必要的脱敏外部动作与结果，不保存隐藏推理或真实私有会话。
- 不修改真实宿主配置、不发送消息到用户已有聊天、不依赖不可用桌面自动化。实际入口不可用时记录能力缺口，不用协议通过冒充已接入。

验收：
- NX04-A01：默认无网络、无凭据仍可完成runner/评分器/故障模拟；真实运行开关默认关闭。
- NX04-A02：真实生产者产生可检查原生事实/来源/检查点；不是脚本直接塞入最终答案。
- NX04-A03：消费者为新会话，无生产者原始对话输入，确实通过原生入口读取并保留未完成事项。
- NX04-A04：历史、环境差异和dirty-worktree场景不会把过期知识当当前通过；正确unknown被奖励。
- NX04-A05：有向宿主矩阵逐格列fixture/protocol/live状态与证据；缺失行不能计入通过数。
- NX04-A06：预算、模型可用性、超时、输出不合法与评分失败分开；不静默换模型。
- NX04-A07：原始响应可离线重评分，oracle不进模型输入，日志不含凭据和真实私有内容。
- NX04-A08：R14-A05和总验收只有在声明范围真实达标后才PASS；只跑一项或两方向则保持限定范围，不泛化为四宿主全通过。

## 4. 共同工程边界与最终交接

所有任务复用Rust内核、Markdown知识正文、规范JSON元数据与现有恢复协议；SQLite保持派生缓存。不要为了本轮新建daemon、图数据库、通用队列平台或第二个语言内核。

测试使用临时运行目录和独立store；保留正确CARGO_HOME/RUSTUP_HOME以使用已安装工具链。不改真实记忆、模型启用策略和宿主配置。未授权时不push/tag/publish/install，不调用模型，不下载资产。

基础门禁保持：
```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
cargo run --locked -- eval run --min-recall 0.95
cargo run --locked -- eval run --pipeline full --min-recall 0.95
cargo run --locked -- eval run --longmemeval --pipeline full --min-recall 0.95
```
以上需用隔离包装器执行，cargo manifest定位工作区但运行库位于临时目录；另外保留transport/HTTP mock、打包白名单和升级恢复门禁。新评分测试通过无网络CI运行。完整LongMemEval和真实模型测试没有运行就如实标NOT_RUN。

逐任务交接必须包含：HEAD/dirty文件；修改范围；逐验收ID的PASS/FAIL/PARTIAL/NOT_RUN/BLOCKED；实际命令和退出码；源/二进制/语料哈希；副作用；恢复/回滚说明；下个依赖已满足任务。

每次状态更新同时维护ACCEPTANCE.md、BACKLOG.json、EXECUTION_LOG.md及MANIFEST；最好增加离线一致性检查，防止正文写PARTIAL、汇总却写all passed。不能改掉原验收文字掩盖没做的范围。历史证据保留原SHA，不把旧CI成功覆盖成当前修改的结果。

首批交付到NX02即可形成一个可审查边界。NX03/NX04不因外部模型不可用而阻塞已完成代码，但也不能因此宣布剩余验收通过。
