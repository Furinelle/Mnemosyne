# Local acceptance and advisory follow-up

Acceptance IDs are preserved; NX03-A04 reflects the user's personal-use scope correction, with its original requirement retained in BACKLOG.json. NX03/NX04 retain PARTIAL against the original broader scope. Final evidence is under `evidence/pro-followup/`; earlier `evidence/final/` is a separate first-batch checkpoint. Remote CI, actual ONNX models and real hosts remain NOT_RUN. The user removed 10000-record evaluation from scope; it is not an outstanding requirement.

## NX00 — PASS

- **NX00-A01: PASS** — 记录 HEAD、源文件摘要、binary hash、toolchain、实际命令和退出码；作者旧日志与本次执行分列。
  Baseline metadata and final manifest distinguish clean a0da375 from the uncommitted source digest.
- **NX00-A02: PASS** — 升级库的正向写入与重放通过，读/预览不写，旧 writer 保护仍在。
  Upgraded ingest/preview/replay and four Stop hosts assert persisted facts and provenance; direct fail-safe session_end reads saved data.
- **NX00-A03: PASS** — 结构化值/环境/多值四类对照正确，旧事实无未经授权失效。
  Reconciliation value/environment/multivalue and no automatic replacement regressions remain enabled.
- **NX00-A04: PASS** — 修订前后来源绑定和 historical prefix 正确，旧 unknown 保留 unknown。
  Revision-bound support, old unknown bindings, history and tamper tests remain enabled.
- **NX00-A05: PASS** — 原有 default/all-features、fmt/clippy、三个检索门禁不降低；环境阻塞不伪装 PASS。
  Existing offline native acceptance gates retained. Final results.json is authoritative.

## NX01 — PARTIAL

- **NX01-A01: PASS** — 候选组三个字段全错、没有工具调用时 quality FAIL、acceptance_passed=false。
  Supplied all-wrong counterexample fails every arm despite no tool use.
- **NX01-A02: PASS** — 候选仅 next_action 错也不通过，不能被 facts_correct=2 掩盖。
  Wrong next action and false refusal fail candidate quality independently of correct database/port.
- **NX01-A03: PASS** — without_memory 准确 unknown 可通过拒答要求，猜中隐藏答案也不算有证据回答。
  Only exact allowed unknown expressions pass without-memory; guessed hidden values are unsupported.
- **NX01-A04: PASS** — v1_retrieval 的 database/port 错误计入；没有提供 next_action 时编造操作被识别。
  Every baseline field is scored against supplied evidence; invented next action fails.
- **NX01-A05: PASS** — 对照正确答案全部通过；与规范化契约不等价的回答不会被模糊字符串匹配放过。
  Only NFKC/case/whitespace normalization and explicit refusal aliases are allowed.
- **NX01-A06: PASS** — 混有 unknown 与肯定断言、非法 JSON、少字段、超预算/非法工具调用都有独立失败用例。
  Malformed/schema-invalid/duplicate-key/non-UTF8 output, false unknown, tools and budgets are rejected.
- **NX01-A07: PASS** — 超时/模型不可用/认证失败是执行错误或 NOT_RUN，不是正确性 PASS。
  Timeout, missing authentication/model/CLI and subprocess failure cannot produce quality PASS.
- **NX01-A08: PASS** — 未执行的 arm/缺失结果不能使总验收通过；报告明确 completed_arms/required_arms。
  Missing or duplicate arms fail acceptance; completed and required arm lists are separate.
- **NX01-A09: PARTIAL** — 评分回归在默认 CI 实际运行，不要求凭据，不调用模型；保留本包最小反例作为负对照。
  Offline test passes locally and is wired into both default CI matrix hosts. Remote CI was not run because this branch is unpushed; do not claim remote execution.

## NX02 — PASS

- **NX02-A01: PASS** — 45×100000 字节语料分页完成，无重复遗漏；原有重放回归保持。
  45 large records still page under 4 MiB with unique coverage and increasing progress.
- **NX02-A02: PASS** — 25个以上合法提案分多次提交完成，每次不超现有上限，均为 pending。
  30 distinct proposals across three output pages remain pending; no page exceeds 20.
- **NX02-A03: PASS** — 总报告超过64 KiB时分片保存/返回；输入游标不提前标完成。
  Aggregate metadata reports exceed 64 KiB; each receipt is bounded and cursor stays uncommitted until the final output.
- **NX02-A04: PASS** — 在清单、提案页、报告、游标的各持久阶段注入失败后，可重试且不重复应用。
  Four journal interruption stages plus all four materialization prefixes recover/replay; legacy report/cursor interruption also repairs.
- **NX02-A05: PASS** — 并发两个客户端提交同一输出页，幂等或明确冲突；不能互相覆盖游标。
  Two concurrent identical submissions return identical receipts. Page content/order conflicts and pending-run cursor bypass are rejected.
- **NX02-A06: PASS** — 单条超大/损坏记录仍拒绝，其他合法记录可继续；汇总 partial/blocked，原文和 core 不变。
  Oversized, invalid UTF-8 and invalid-ID records become blocked references; valid inputs continue and final status remains partial.
- **NX02-A07: PASS** — 快照源发生语义更改后拒绝旧结果；访问热度与显式输入修改按已公开契约处理。
  New/uncommitted stale results fail; exact store-bound committed receipt replay survives source changes without mutating proposals or newer progress. Strict access-metadata invalidation remains documented.
- **NX02-A08: PASS** — 越界路径、symlink/hardlink、伪造游标、非法序号仍被拒绝，不把安全限制删掉。
  Input/report symlink and hardlink tests, forged snapshots, scope checks and invalid output indices remain fail-closed.
- **NX02-A09: PASS** — 终页为空、limit极小、输出零提案、超过声明库规模均有确定终止行为与状态。
  Empty terminal, one-entry pages, no-proposal output and 10,001-file hard-cap tests have deterministic results.
- **NX02-A10: PASS** — R11-A05 只有在相应原要求覆盖后才升级 PASS；保留的硬限制或范围偏差在 ADR/ACCEPTANCE 明示，不自行认定例外等于原要求全满足。
  ADR documents the retained 10,000-file, 128 KiB record and 1,000-output-page ceilings; historical R11-A05 stays PARTIAL.

## NX03 — PARTIAL

- **NX03-A01: PARTIAL** — 报告分别列legacy/V2库schema、记忆/来源/修订/检查点数量，基准确实走正式V2路径。
  Formal V2-current/evolution reports record schema, memories, sources, revisions and checkpoints. Legacy recipe is retained and smoke-tested; no new large legacy comparison was run.
- **NX03-A02: PASS** — 每组报告command/commit/binary hash/corpus hash/环境/raw samples/实际lane，非目标fallback明确标出。
  Per-command raw timing, generator recipe hash, actual corpus hash, binary hash, environment and observed lane are recorded. Final MANIFEST binds the uncommitted source set to reports and binaries.
- **NX03-A03: PASS** — 正向Stop确实产生新V2事实，prep确实包含完整checkpoint next_action；空输出不能当快速成功。
  Native Stop creates a new persisted V2 fact; prep contains the expected checkpoint action. Six-path native fixture check runs in acceptance and CI.
- **NX03-A04: PASS** — 自用范围：包含历史与证据的100/1000条V2语料完成；不要求10000条评测。
  Both V2 profiles at 100 and 1000 records passed. The user explicitly removed the 10000-record benchmark requirement for personal use.
- **NX03-A05: PARTIAL** — 独立记录无模型和已授权ONNX场景，inclusive计时不双重计算。
  No-model lexical and host-distill paths measured; inclusive phases stay separate. Actual ONNX model measurement NOT_RUN; feature compilation is not model execution.
- **NX03-A06: NOT_RUN** — 任何优化前后，外部编辑、同大小替换、rename/删除、并发、坏缓存、未决恢复结果一致。
  No performance optimization was made, so there is no before/after optimization comparison. Existing integrity regressions remain in the 181-test gates.
- **NX03-A07: PARTIAL** — 性能回退按同条件对照披露；不要求为达速度目标删除安全检查或改变语义。
  New V2 numbers are initial descriptive baselines, not a controlled speedup/regression comparison. Safety and semantics remain unchanged.

## NX04 — PARTIAL

- **NX04-A01: PASS** — 默认无网络、无凭据仍可完成runner/评分器/故障模拟；真实运行开关默认关闭。
  Default deterministic offline runner uses isolated synthetic stores and fixed native CLI/MCP commands, without credentials or model calls. Negative runner tests are wired into CI.
- **NX04-A02: PARTIAL** — 真实生产者产生可检查原生事实/来源/检查点；不是脚本直接塞入最终答案。
  Real native interfaces persist synthetic producer facts/sources/checkpoints; Stop persists the business fact and controller rereads durable state. Real authenticated host/model producers NOT_RUN.
- **NX04-A03: PASS** — 消费者为新会话，无生产者原始对话输入，确实通过原生入口读取并保留未完成事项。
  New consumer process reads prep/show via native CLI or MCP. It receives no producer receipt, transcript, checkpoint JSON or oracle. This is fixture contract acceptance only.
- **NX04-A04: PARTIAL** — 历史、环境差异和dirty-worktree场景不会把过期知识当当前通过；正确unknown被奖励。
  Eight fixture scenarios cover unresolved failure, revision-bound sources, same-commit dirty worktree revalidation and wrong-project unknown. Broader real-host environment comparisons NOT_RUN.
- **NX04-A05: PARTIAL** — 有向宿主矩阵逐格列fixture/protocol/live状态与证据；缺失行不能计入通过数。
  Exactly CLI-to-MCP and Stop-fixture-to-CLI across four cases are required. Missing/duplicate rows fail. No four-host matrix completion is claimed.
- **NX04-A06: PARTIAL** — 预算、模型可用性、超时、输出不合法与评分失败分开；不静默换模型。
  Timeout, malformed output, output limit and quality failures cannot pass. Model availability/cost paths NOT_RUN. Some malformed nested output is conservatively recorded as invocation failure.
- **NX04-A07: PARTIAL** — 原始响应可离线重评分，oracle不进模型输入，日志不含凭据和真实私有内容。
  Successful observations retained in JSON for offline checks; controller-only random canary is not passed to workers. No OS sandbox or complete archival of malformed raw responses is claimed.
- **NX04-A08: PASS** — R14-A05和总验收只有在声明范围真实达标后才PASS；只跑一项或两方向则保持限定范围，不泛化为四宿主全通过。
  Acceptance is explicitly offline_native_contract for two fixture directions. Historical R14-A05 remains PARTIAL and model/live-host fields stay NOT_RUN.

