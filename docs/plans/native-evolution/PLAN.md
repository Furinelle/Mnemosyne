# Mnemosyne Rust 原生演进实施计划

计划标识：Native Evolution R1。编写日期：2026-09-19。
目标仓库：Furinelle/Mnemosyne。核查基线：`d05f02853f42771ae9ca04d78b79739d01f1a76c`，Cargo version `1.0.0`。
执行者：Codex / Astra。本文是待实施计划，不是完成报告；所有新增命令、类型、目录均为设计提案，除非明确注明“已有”。

## 0. 执行摘要

Rust 迁移已完成，本计划取消“Python 业务层保留”“Rust probe 研究”“再次实现相似度止损/整包预算/embedding fingerprint/图扩展/关系恢复”等过期任务。下一步是：**原生小缺口收口 → 来源与任务接续 → 语义历史及审批 → 可追溯归纳**。性能和验收贯穿始终，而非等所有功能完成后才测。

推荐第一批只完成 R00、R01、R02、R03，并取得 R14 的基线；第二批优先 R04→R05→R06，使换 agent 后不丢进度。R07 与 R05/R06 在接口冻结后可分支推进。不要让 TaskCheckpoint 等待整个历史系统，也不要在历史/审批基础之前先上 sleep。

版本建议而非承诺：修复批可作为 1.0.x 候选；来源/上下文/检查点可作为下一 minor。涉及公开 Rust API 或持久格式不兼容时按实际契约决定版本，不能为了预设的 1.1/1.2 编号掩盖 breaking change。此计划不授权发布、安装或修改真实宿主配置。

## 1. 证据范围与限制

本轮通过 GitHub 连接固定提交读取 Cargo、README、关键 Rust 模块、迁移文档和 CI 状态，并重新打开前文参考仓库的公开主页/有关文档。核查时该提交 CI run `35441992545` 为 success。[S01] [S15]

本轮没有执行 Rust 编译或测试：分析容器未找到 Cargo，直接 clone 因 DNS 无法解析失败。代码风险为静态审阅结论，R00/R01/R02/R03 要先用真实 Rust 回归复现；不得说本轮已跑过。仓库本机验收报告记录的 44 项测试与真实 ONNX 是作者对特定快照的记录，不是本轮新执行证明。[S13]

外部项目只借鉴设计，不声称审计全部源码、不提供竞品跑分排名、不复制许可证未核实的代码。所有代码复用须在实施时固定原仓库 commit 和文件许可证；无法核实则独立实现。当前任务的权威顺序：用户最新要求 → 实际工作区源码和测试 → 本计划基线 → 旧 Python 路线；若 HEAD 已推进，先更新差异表而不是倒退到本提交。

## 2. 已有能力与真正增量

| 主题 | 固定提交现状 | 本计划处理 |
|---|---|---|
| 运行时 | Rust 1.0.0；edition 2024/MSRV 1.95；Python API/Hermes provider 退役 | 不重做语言迁移，不保留退役接口 |
| 保守写入 | 精确 body/type/tags/expiry/evidence 比对；不同事实不因相似自动失效 | 保留设计，补 source 来源遗漏与 provenance |
| 上下文预算 | assemble 计全包；estimated；core 超限报错 | 只增结构化解释、--budget、path/epoch/revision |
| 向量一致性 | input hash/model fingerprint/dimensions/回填后 CAS 已有 | 补坏行自愈、查询推理一致性与诊断 |
| 会话去重 | host/channel/session 和 scope/path/id 已区分，锁与 TTL 已有 | 新增内容更正与压缩周期感知 |
| 关系写入 | 两文件、跨 store 有持久恢复协议 | 复用，不当作完整 revision history |
| 检索 | FTS5/CJK/vector/RRF/link expansion/rerank | 做消融和性能，不重复开发 graph-hopping |
| 宿主 | Codex、Claude Code、Grok Build、Antigravity；MCP project_path | 保持当前四宿主，先协议测试再可选实机 |
| 生命周期 | 仍 per maintenance run 衰减 | 先守住失效状态，再显式引入 per_day |
| 时态/检查点/审批 | 当前 CLI/核心格式未提供完整这些能力 | 新任务，不能在 README 先写已支持 |
| 文档 | rust-migration 已更新，interface 仍残留旧 Python 描述 | R00 对齐，不删除历史记录 |

依据：[S02](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/Cargo.toml)；[S03](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs)；[S05](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs)；[S06](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/vectors.rs)；[S07](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/models.rs)；[S10](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs)；[S11](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/relations.rs)；[S12](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-migration.md)；[S16](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/interface.md)。

## 3. 不可破坏的设计约束

1. 原生 Rust 是唯一内核；开发用 Python stdlib smoke 驱动可以保留，不是运行时依赖。继续保持 CLI/MCP/四个中立事件；旧 CLI/MCP aliases 按现有文档保留。
2. Markdown 是知识原文；身份、证据、历史/提案 manifest 等 JSON 是新增规范持久元数据，不是可扔缓存。只有 `rust-index.sqlite`、`vectors-rust.sqlite` 和明确标识的投影可重建删除。
3. 默认不需 LLM、图数据库、daemon。模型资产与 ORT 显式提供；不自动下载、不调用真实付费 endpoint、不改用户全局 enablement。
4. 网络端点、凭据变量、模型路径、监听配置保持全局信任边界。仓库配置/记忆/提案里的文字不能变成执行权限或任意 shell。
5. 所有写操作保留既有 fs2 锁、原子写、恢复、路径白名单和 symlink 检查；模型调用不得持有 store lock。
6. 不让 strength/召回次数充当真实性；同源转述不是独立佐证；fact_key 一样不等于可自动 supersede。
7. 不对真实 ~/.mnemosyne、实际项目记忆、用户已有聊天或宿主配置执行测试写入；本项目计划不授权 push/tag/publish/安装。
8. 旧文件可读不等于旧 writer 可安全写新格式；新 schema、旧 writer、pending coordinator 的升级限制必须显式说明。
9. 不借此“清理所有代码”。保留已验证模块，按一个可验收垂直切片实现；新模块仅服务明确职责，不建立泛型插件平台或第二套事务引擎。
10. 不把恢复日志当历史，不把源码 Git commit 当运行环境真相，不把协议测试当真实模型/宿主端到端证明。

## 4. 分阶段目标和依赖

| 阶段 | 任务 | 交付 |
|---|---|---|
| M0 原生收口 | R00–R03；R14 基线 | 契约/来源小修/向量自愈/协议错误边界 |
| M1 可用性提升 | R04–R06 | 稳定来源、解释性上下文、可靠跨 agent 检查点 |
| M2 可信演化基础 | R07–R10 | 持久修订、历史内容、受审 reconciliation、幂等维护 |
| M3 审批式整理 | R11–R13 | 有界 sleep、带来源视图、安全快照恢复 |
| M4 候选发布验收 | R15；R14 贯穿各阶段 | 实测兼容、质量、性能与恢复文档 |

机器依赖以 BACKLOG.json 的 depends_on 为准。允许独立 R01/R02/R03 分支及测试并行；同一文件/持久 schema 的集成需串行。不能因独立任务顺序不同就降低验收。

## 5. 固定的数据契约

### 5.1 身份与来源

`MemoryRef = (store_id, memory_id)`。旧裸 id 仍有原生 v1 行为；新 v2 显式限定作用域。
`IdempotencyKey = (store_id, origin, session_id, source_event_id, finding_key)`，payload_hash 独立记录；一个 event 可产生多条 findings。没有稳定来源 key 的旧文本写入继续 conservative exact path，不能虚构来源。

`source_kind` 表示内容来源，`verification_state` 表示核验程度。两个字段不能合成一个 confidence 数字。外部证据可以失联，此时显示 unavailable，而非重写为 verified。source_event_id 与 SourceEvent 在多个转述间保持来源链。

### 5.2 三种哈希/版本不能混用

- `semantic_rev`：body、约束、关系、状态、来源与证据语义变更。
- `embedding_input_hash`：严格哈希实际给 embedding 的字符串；访问热度不在输入中。
- `disk_hash`：提交/恢复时检查文件 before/after，不能以 semantic hash 代替字节冲突检查。

访问统计不是语义历史。新增 `Clock` 注入用于可重复时间测试，避免 sleep、TTL、时间查询各用一套不可控制系统时钟。

### 5.3 建议的增量持久布局

以下新目录必须在对应任务实施后才启用，不要求一次性创建：

```text
.mnemosyne/
  core.md                         # 已有，人工核心规则
  working/*.md                    # 已有，当前事实
  archive/YYYY-MM/*.md            # 已有，归档事实
  MEMORY.md                       # 已有，派生人类目录
  rust-index.sqlite               # 已有，派生词法缓存
  vectors-rust.sqlite             # 已有，派生向量缓存
  .relations-operation.json       # 已有，未决操作，不是历史
  .relations-commits/             # 全局 coordinator，已有
  store.json                      # R04，身份/schema/min_writer_version
  evidence/*.json                 # R04，最小来源/核验元数据，规范数据
  checkpoints/*.json              # R06，短期任务状态，规范数据
  history/<id>/<revision>.md       # R07，持久语义镜像，规范数据
  history/<id>/manifest.json      # R07，系统时间/提交序列
  proposals/*.json                # R09，提案/批准/应用记录
  reports/sleep/*.json            # R11，最小审计报告
  views/*.md                     # R12，可再生的带来源视图
```

配置和 schema 只 additive 时也要考虑老 writer 的重写行为。未知 flat 字段保留；复杂嵌套不套用旧 frontmatter 支持假设。历史镜像与 provenance sidecar 不能仅放 SQLite 中。

### 5.4 权限矩阵

普通 agent：可按宿主策略新增记忆/检查点、读取、提出提案。自动规则：可验证前置条件后维护派生缓存和确定性重放。语义合并、因果连边、supersede、core 改写：明确提案/授权+CAS。删除/purge/真实恢复：单独用户授权。本计划不新增无人值守 destructive 开关。

## 6. 任务实施卡

每张卡均需先添加失败回归再实现。验收条目是待运行要求，不是已通过声明。代码落点是目前实际文件；新增文件会明确标注。

### R00 — 锁定 Rust 基线、契约和可重复验收
优先级：P0；阶段：M0；依赖：无。
目标：以实际工作区和固定 Rust 提交为基线，不把旧 Python 路线机械翻译成 Rust。
落点：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`.github/workflows/ci.yml`、`docs/interface.md`、`docs/adapters.md`、`docs/handoff-format.md`、`docs/rust-migration.md`、`tests/*.rs`、`tests/native_*_smoke.py`。
实施：
1. 记录 HEAD、dirty diff、rustc/cargo 版本、host triple、Cargo features、测试/评测语料 SHA。HEAD 与本计划不同时先写差异表：已实现、仍缺、风险改变；不 reset 用户工作区。
2. 保存当前 CLI --help、JSON 字段、八个 MCP 工具及旧 aliases 的契约快照。新功能应 additive；如实际承诺无法兼容，写迁移说明，不假称完全兼容。
3. 修正文档仍出现的 Python import、portalocker、旧 index.sqlite、旧目录和退役 Hermes API。保留历史验收记录，给它们加适用提交，不能把旧记录改成新提交的通过证明。
4. 隔离 HOME/MNEMOSYNE_HOME、测试项目、模型配置和写入目录；保留正确 CARGO_HOME/RUSTUP_HOME。对测试驱动子进程使用 env_clear/显式白名单，不在多线程 Rust 测试中修改共享进程环境。
5. 记录固定提交 CI run 35441992545 的成功状态；复跑当前工作区门禁，不把远端 success 当本地改动的测试结果。创建 docs/plans/native-evolution/EXECUTION_LOG.md。
6. 确认 cargo package --list 的白名单；不含真实记忆、模型、密钥、开发机缓存。先保留最低 MSRV 1.95 和 edition 2024；不以降级工具链作为本轮任务。

验收：
- [ ] **R00-A01** 所有原有原生测试、fmt、clippy 和现有三个检索门禁实际执行并记录退出码；受限环境明确 not_run。
- [ ] **R00-A02** 以同一 JSON fixture 调 CLI/MCP 后可解释字段差异；八个工具及两个 CLI alias/一个 MCP alias 仍可用。
- [ ] **R00-A03** 测试不新增、修改或读取测试范围外的真实记忆内容；原生运行测试清空 PATH 仍可工作。
- [ ] **R00-A04** 新文档的默认缓存名是 rust-index.sqlite / vectors-rust.sqlite；不宣称 Python runtime/API 或 Hermes provider 仍受支持。
- [ ] **R00-A05** 打包清单没有 .mnemosyne、credentials、models、target、私有报告；源包解包后能离线使用已缓存依赖构建，缺缓存时说明限制。
- [ ] **R00-A06** 内置 50-query 检索门禁和 2-case LongMemEval 样本分别标识，未声称跑了完整公开基准。

不做：不改写内核架构、不自动升级依赖、不安装二进制、不更改真实宿主设置。
回滚：仅撤回本任务新增的契约/文档/测试；保留用户既有 diff，不执行破坏性 git 回滚。
参考：[S01](https://github.com/Furinelle/Mnemosyne/commit/d05f02853f42771ae9ca04d78b79739d01f1a76c)；[S02](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/Cargo.toml)；[S13](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-local-cutover.md)；[S14](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/.github/workflows/ci.yml)；[S15](https://github.com/Furinelle/Mnemosyne/actions/runs/35441992545)。

### R01 — 补来源保留与生命周期语义回归
优先级：P0；阶段：M0；依赖：R00。
目标：在已有保守写入基础上，修补剩余小缺口；绝不恢复 Jaccard 自动 supersede。
落点：`src/api.rs`、`src/schema.rs`、`src/ingest.rs`、`tests/cli.rs 或实际 CLI 集成测试文件`。
实施：
1. 先复现 duplicate_entry 未比较 source：其余字段相同但 source 不同的写入可能只返回旧 ID。第一小补丁比较规范化后的有效 source（空值等同 agent）；不同来源至少保留两条，不丢新来源。R04 上线后可用 SUPPORT 合并证据，不把来源数量当独立证据数量。
2. 保留 type/body/tags/expires/evidence 的现有精确重复语义和 allow_duplicate 逃生口；不要随意排序 tags 或归一化正文造成新的语义去重。
3. 用显式 fixture 复现 maintain 对低 strength superseded 记录可能写为 deprecated。状态失效原因与热度分类应分开；维护不得移除 invalidated_by、复活已替代事实或将它列为 core candidate。
4. 保持 expired 过滤和旧按次 decay 行为。R10 才引入新按天模式，本任务不偷偷改变维护节律。
5. 把每一个行为改变沿 CLI/MCP/ingest/distill 最终共享的 write_entry 路径验证；不在适配器里各补一份去重。

验收：
- [ ] **R01-A01** 同 source 精确重放只有一个有效记录；仅 source 改变时第二来源不丢失。
- [ ] **R01-A02** 同标题的 port 8080 与 healthcheck /health 共存；改成 port 8081 仍保留新事实。
- [ ] **R01-A03** 长正文只变一个版本号/否定词不被判 duplicate；evidence 或 expires 更新仍能保存。
- [ ] **R01-A04** 使用低 archive 阈值保留在 working 的 superseded 记录，多次 maintain 后仍为 superseded 且默认搜索不可见。
- [ ] **R01-A05** 归档后的 superseded 记录在 --archive 且不含 include-superseded 时不会因为状态改写重新出现。
- [ ] **R01-A06** 高 strength、高 access_count 的 superseded 记录不成为 core candidate；旧 valid 记录维护行为没有意外变化。

不做：不重建七决策系统、不修改真实库、不批量纠正历史误判。
回滚：此补丁不需要 store migration；撤回代码前保留新来源记录，不删除用户已写知识。
参考：[S03](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs)；[S04](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/schema.rs)。

### R02 — 向量缓存自愈与推理快照一致性
优先级：P1；阶段：M0；依赖：R00。
目标：保留现有 fingerprint/hash/dimension/CAS；补坏行可恢复性和查询时配置一致性。
落点：`src/vectors.rs`、`src/models.rs`、`src/search.rs`、`src/main.rs`、`tests/native_http_models_smoke.py`。
实施：
1. 复现 cached_vectors 会忽略坏向量，但 cached_hashes 只看 content_hash/fingerprint/dimension，导致 backfill 可能永远跳过同一坏行。让新鲜性同时要求可解码、维数正确、有限且非零。
2. 使 backfill 的每批缓存提交有事务和结构化统计：computed/written/skipped_stale/invalid/repaired/failed；旧 CLI 摘要可保留，新增 --format json。
3. 查询 embedding 与缓存匹配使用同一个不可变 ModelProfile。查询计算期间模型配置/本地资源改变时丢弃该次向量结果，保留 lexical lane；不能仅对 backfill 做 CAS。
4. 保留模型调用在 store lock 外。用注入式 fake provider 复现竞争，不靠随机 sleep。缓存缺失、损坏、不可写时不能导致所有基本搜索不可用。
5. 诊断明确区分 disabled / missing_assets / incompatible / corrupt / stale / ready。远端同名 alias 背后的未公布权重变化不可被本地哈希自动识别；支持用户显式 revision，不作绝对保证。
6. 当前 fingerprint 逐次读取本地模型和词表；R14 先测量，只有有证据才缓存。缓存优化不得只凭 path/size 就假称等价于内容哈希。

验收：
- [ ] **R02-A01** 把一条原本新鲜缓存改为零向量、错误维数或畸形 JSON 后，下次 backfill 修复它，而不是永远跳过。
- [ ] **R02-A02** 正常同输入、同模型的重复 backfill 不调用 provider；只改变 access_count 不重算。
- [ ] **R02-A03** 推理期间正文、dimensions、model/revision 改变时旧结果不落为新鲜向量。
- [ ] **R02-A04** 查询推理期间 profile 改变时旧缓存与新查询向量不混用；词法结果仍返回且有降级诊断。
- [ ] **R02-A05** 断开 ONNX/HTTP 或损坏向量 SQLite 时，未启用模型的基本读写/MCP 不受影响。
- [ ] **R02-A06** provider 返回乱序批次、重复 index、NaN、Inf、零向量、错误维数均有明确处理；真实模型验收与 mock 分开。

不做：不再新建另一套向量索引、不默认下载模型、不接入额外云服务。
回滚：向量 SQLite 是派生数据，可在持锁、停用写入后重建；不能删 Markdown/证据/历史。配置迁移采用 additive。
参考：[S06](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/vectors.rs)；[S07](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/models.rs)；[S08](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/search.rs)；[E07](https://github.com/memvid/memvid)。

### R03 — MCP 原生协议与有界输入验收
优先级：P0；阶段：M0；依赖：R00。
目标：把“能列出八个工具”推进为错误、通知、大小上限和项目作用域都符合声明协议。
落点：`src/mcp.rs`、`src/mcp_tools.json`、`src/main.rs`、`src/adapters.rs`、`tests/native_only.rs`、`tests/native_transport_smoke.py`。
实施：
1. 以官方 MCP/JSON-RPC 为准审阅 process_line：当前只特别忽略 notifications/initialized，initialize 固定返回 2024-11-05。固定旧版并非自动是缺陷；应明确支持集合、协商和连接生命周期，不盲改版本号。
2. 明确区分协议错误（parse、非法 params、未知 method）与工具执行业务错误。按声明版本的工具结果契约返回 isError，不把“记忆不存在/权限拒绝”等都混为 -32603。
3. 对无 id 通知、非法 id/jsonrpc、初始化顺序、未知通知做回归。不能因加入兼容路径而执行畸形写请求。
4. SSE 已有 1MiB 请求限制和有界会话/队列，保留；为 stdio line/main stdin 路径补一致的可配置大小上限。超限有明确错误/断连策略，不积累无界内存。
5. 保留 project_path 从绝对项目解析、每请求隔离、global/project exposure。运行本地服务不等于对不可信本机进程/互联网提供强 ACL；不得把 project_path 当授权。SSE 默认仅本机；非本机监听需要显式安全策略，不能以 session_id 充当认证。
6. 能力/错误/工具 schema 由单一 Rust 类型或注册表生成/验证，避免 schema 与 dispatch 各漂移一份。审批类能力默认不向普通 agent 自动暴露。

验收：
- [ ] **R03-A01** 正确初始化→工具列表→调用→断开能在 stdio 和已有 SSE 运行，CLI aliases 不受影响。
- [ ] **R03-A02** 无 id 通知不产生伪造 id:null 响应；非法请求不写 store；具体允许的通知记录在契约。
- [ ] **R03-A03** 工具业务失败以所声明协议的工具错误结果返回，parse/params 错误仍是 JSON-RPC 错误。
- [ ] **R03-A04** 超大/非法 UTF-8/负 limit/错误类型/未知字段均不导致进程 panic、越界写或无界缓冲。
- [ ] **R03-A05** 同一服务器在 cwd=/ 交替读写两个显式项目，不串库；关闭 global exposure 后每个相关工具都遵守。
- [ ] **R03-A06** 没有模型库、Python、外网和 HOME 中旧包时，八个基础工具仍满足其非模型契约。

不做：不以协议修正为由重写整个 server 或引入常驻服务框架；不自动装宿主插件。
回滚：保留协议契约 fixture；行为纠正可能影响非规范客户端，提供错误兼容说明，不恢复不安全写入。
参考：[S09](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/mcp.rs)；[S10](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs)；[E12](https://modelcontextprotocol.io/specification/2024-11-05/server/tools)；[E13](https://modelcontextprotocol.io/specification/2024-11-05/basic/lifecycle)。

### R04 — 稳定身份、来源事件和写入结果 v2
优先级：P1；阶段：M1；依赖：R01, R03。
目标：让事实身份与证据来源不再依赖“代理名+截断字符串”，仍保持原始 Markdown 可读。
落点：`src/schema.rs`、`src/store.rs`、`src/api.rs`、`src/ingest.rs`、`src/main.rs`、`src/mcp_tools.json`、`新增 src/provenance.rs`。
实施：
1. 先写 ADR：StoreManifest(schema_version,store_id,min_writer_version)、MemoryRef(store_id,memory_id)、SourceEvent、WriteRequestV2/WriteOutcome。路径是本机定位而非唯一持久身份；store_id 不是身份认证。
2. 保留旧裸 ID 入口；新入口允许限定 store，多个候选时 v2 显式返回歧义。不得静默把旧 first-match 语义全局改成另一行为；新旧差异通过契约版本公开。
3. 新增 flat frontmatter：recorded_at、source_event_id、source_session_id、source_kind、verification_state、fact_key（可选）。复杂 evidence 集合用版本化 JSON sidecar 和稳定 ref，禁止凭现有受限 parser 随意输出嵌套 YAML。
4. knowledge Markdown + 证据/身份 JSON 是规范持久数据；它们不属于可删除索引。SQLite 只索引这些数据。为新 schema 设置写入兼容检查；旧原生 writer 未实现识别时，只能通过停旧 writer、升级入口和备份来避免混写，不声称自动阻止它。
5. 幂等键限定 origin/session/event/finding：同一事件可含多条发现，不能只凭 source_event_id 丢掉后续 findings。同幂等键同 payload 返回既有结果；同键异 payload 明确冲突，不覆盖。
6. 区分 source_kind（user_statement/tool_output/code_observation/agent_inference）与 verification_state（unverified/evidence_attached/verified）。verified 只能由明确的可信核验流程设置，不能因 agent 自填或被多次召回而升级。
7. 默认不保留原始敏感 transcript；只存最小来源引用、内容哈希与可选脱敏摘要。SUPPORT 追加证据仍保留共同源事件关联，不宣称统计独立。
8. 引入可注入 Clock；recorded_at 用 RFC3339 UTC，保留旧 created 日期但不将其推断为精确历史时间。

验收：
- [ ] **R04-A01** 两 agent 转述同一源事件，不增加独立证据；新独立事件支持相同事实时证据不丢。
- [ ] **R04-A02** 同一源事件产生两条不同 findings，都能保存；并发重放每条仅提交一次。
- [ ] **R04-A03** 同幂等键不同 payload 明确 IDENTITY_CONFLICT；重试不静默更换结果。
- [ ] **R04-A04** 未知 flat frontmatter、中文、引号、逗号、反斜线 round-trip；legacy 文件不被一次读操作全量改写。
- [ ] **R04-A05** agent 自填 verified 不获可信验证等级；来源缺失输出 unknown，不伪造 user_asserted。
- [ ] **R04-A06** 具有同裸 ID 的两个 store 可通过新 MemoryRef 正确定位；旧入口行为记录且可测试。
- [ ] **R04-A07** 新旧 JSON、MCP v1/v2 契约可区分；真实配置不变，schema 首次升级可预览和在副本上回滚。

不做：不强制把每条自然语言变为三元组、不实现团队认证、不加入 LLM 必选依赖。
回滚：先备份并验证恢复；sidecar/manifest 只 additive。回滚到旧 writer 使用迁移前副本，不让旧 writer 改写启用新语义的 store。
参考：[S03](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs)；[S04](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/schema.rs)；[S12](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-migration.md)；[E01](https://github.com/mem0ai/mem0)；[E02](https://github.com/getzep/graphiti)；[E10](https://github.com/ourmem/omem)。

### R05 — ContextBundle 增量完善：路径、修订与压缩周期
优先级：P1；阶段：M1；依赖：R04。
目标：复用 assemble 的已有整包 estimated budget，不重建上下文编译器。
落点：`src/context.rs`、`src/api.rs`、`src/search.rs`、`src/adapters.rs`、`src/main.rs`、`src/mcp.rs`、`assets/templates/`。
实施：
1. 把现有字符串+selected 内部结果封装为有版本 ContextBundle：context、items、estimated_tokens、budget_mode、selected/omitted 原因。保留旧输出字段，新增 --budget 覆盖默认配置，不能因 budget 参数改全局配置。
2. 现有 session key 已区分 host/channel/session，memory identity 已区分路径和 scope；新增 context_epoch 和语义内容哈希（R07 后替换/补充 semantic_rev），使同 ID 更正和宿主压缩后的必要信息可再次注入。
3. 变更 session 状态版本，保留 TTL/锁/锁争用时不丢召回；使用结构化 key 防字符串碰撞。不把“候选被搜索到”算实际交付。
4. file_touch 先匹配显式 repo-relative related_paths，其次目录/模块，最后 basename + 词法/向量回退；不把代码仓全文作为记忆导入。
5. 保留 relevance-first，不直接相乘 RRF×strength。必要的冲突/来源警告必须计入预算，不能裁掉警告却留貌似确定的断言。
6. 本轮默认 estimated，不承诺特定模型 tokenizer 的严格上限。mandatory core 超预算仍 BUDGET_TOO_SMALL；以后精确 tokenizer 插件是可选扩展，不阻塞此任务。
7. 加入限定数量的 show-many 和按需详情接口可选小补丁；裸全文 show/read 与注入摘要预算分开，不偷偷截断导出原文。

验收：
- [ ] **R05-A01** core+标题+来源+警告+提示+检查点占位全部纳入预算；mandatory 超限沿 fail-safe 输出空 context+stderr。
- [ ] **R05-A02** 同一 revision 同 epoch 不重复注入；同 ID 内容更正或 context_epoch 改变可以再次注入。
- [ ] **R05-A03** 只改访问计数不触发重新注入；另一 host、另一 project、另一 channel 不共享错误抑制状态。
- [ ] **R05-A04** backend/config.rs 与 frontend/config.rs 有区别；找不到完整路径时有可解释 basename fallback。
- [ ] **R05-A05** 缓存/会话锁争用时仍交付可用有界上下文；被裁掉的条目不记 injected/access。
- [ ] **R05-A06** CLI、MCP 和 hook 在相同输入下候选与预算一致，通道差别仅在合法外壳/读取提示。

不做：不重复实现原有预算、不强制第三方 tokenizer、不自动修改宿主 compaction 配置。
回滚：会话去重缓存可删重建；旧字段保留。新 epoch 未提供时兼容现有宿主，不要求所有 host 具备同一种钩子。
参考：[S05](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs)；[S10](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs)；[E04](https://github.com/basicmachines-co/basic-memory)；[E05](https://github.com/thedotmack/claude-mem)。

### R06 — 原生 TaskCheckpoint：真正接续工作
优先级：P1；阶段：M1；依赖：R04, R05。
目标：把一次任务的进度和测试证据，与可复用长期知识分开。
落点：`新增 src/checkpoint.rs`、`src/api.rs`、`src/context.rs`、`src/mcp_tools.json`、`src/adapters.rs`、`docs/handoff-format.md`、`tests/native_only.rs`。
实施：
1. 新增 checkpoints/<uuid>.json，使用独立 schema、revision、单文件原子 CAS。最小字段：task_id/goal/state/completed_actions/artifacts/tests/unresolved/next_action/source_session/observed_commit/worktree_fingerprint/created_at/expires。不为了短期检查点先实现全库历史。
2. new/load/update/close 命令走共享 Rust API，update 带 expected_revision；MCP 扩展先做 versioned schema，默认不增加不必要的工具数量。长期 ingest 不自动将 checkpoint 变成高强度 memory。
3. 每条测试证据记录执行命令、工作目录的项目相对引用、代码状态哈希、退出码、执行时间、报告摘要引用。reported 与 kernel/受信驱动观察到的 observed 区分；缺退出码不能写 passed。
4. 接手时按 task/项目/环境选择少量检查点。代码变化后只说明证据对旧状态有效/需复验，不把旧 pass 抹去，也不升级为当前通过。
5. 工具协议 fixtures 覆盖 Codex/Claude/Grok/Antigravity 的 writer→reader；不恢复已退役 Hermes provider。真实应用验收仅在用户另行授权真实配置时进行。
6. 工作区可能有未提交修改，记录 scoped dirty fingerprint；Git commit 相同不等于工作区相同。不要全盘扫描私有文件。

验收：
- [ ] **R06-A01** A 修改两文件、执行测试且 2 fail，B 接手必须看到未解决的 2 fail，不显示任务已验证完成。
- [ ] **R06-A02** 只有“准备跑测试”的文字不变成 executed/passed；reported evidence 不伪装成受信观测。
- [ ] **R06-A03** 同 commit 但 dirty 文件变化后，测试证据标记需复验。
- [ ] **R06-A04** 两个 agent 并发 update：一个成功，另一个 CAS conflict；不采用 last-writer-wins。
- [ ] **R06-A05** 关闭/到期 checkpoint 不再默认注入，但显式查看可保留记录；不会变成永久 pitfall。
- [ ] **R06-A06** 四宿主 12 个有方向的跨宿主组合使用协议 fixtures 验证身份与交接；与真实模型会话统计分列。

不做：不做任务调度器、不执行 checkpoint 中的任意命令、不把 plan 状态自动改为 done。
回滚：单文件 schema 可导出；禁用 checkpoint 注入即可回退基础工作流；保留检查点原文不删证据。
参考：[S05](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs)；[S10](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs)；[S13](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-local-cutover.md)；[E03](https://github.com/letta-ai/letta)；[E05](https://github.com/thedotmack/claude-mem)。

### R07 — 语义修订与可恢复写入的共同基础
优先级：P1；阶段：M2；依赖：R04。
目标：在已有关系恢复协议上增量扩展；恢复日志不是历史数据库。
落点：`src/relations.rs`、`src/store.rs`、`src/api.rs`、`src/schema.rs`、`新增 src/revisions.rs`、`新增 mutation 模块仅在有必要时拆分`。
实施：
1. 现有 .relations-operation.json / .relations-commits 已有校验和协调恢复，不重写一套并行提交协议。先为它们补公开内部 MutationPlan 接口，保留 v1/v2 journal 恢复。
2. 为 body/type/tags/source/evidence/links/status/expires 等语义状态维护 semantic_rev；access_count/last_accessed/strength 变化不生成语义历史。语义 CAS 与磁盘字节 CAS 分开，防 metadata bump 导致假冲突或吞真实编辑。
3. 新增 history/<memory_id>/<revision>.md 的不可变版本和有版本 manifest；仅成功提交记录系统时间。第一版支持单 store 原子计划，已有跨 store 关系继续原协议，不借机实现通用分布式事务。
4. 保存操作意图→写持久历史镜像→记录提交决定→物化当前 Markdown→标记完成。新建文件必须有 absence precondition；路径白名单和 symlink 防护不得因支持 history/JSON 而放开为任意路径。
5. 在模型/归纳计算前取得输入 snapshot，计算在锁外；提交时检查期望 rev 和来源哈希。所有知识修改入口必须走统一 mutation，包括 link、consolidate、维护中的状态变更。
6. 外部手改：与上次已知镜像比较并在首次观察时记录 external_edit；无法知道真实编辑时刻或中间版本时明确 unknown gap，不用 mtime 冒充系统记录历史。
7. 恢复遇到外部不同内容保留现场并报 RECOVERY_CONFLICT；旧 coordinator 未清空前不搬 store/改变 global root。commit decision 清理需证明所有参与者完成，不能按年龄盲删。

验收：
- [ ] **R07-A01** 新建、更正、supersede、关系变化可重建历史；只访问或调热度不产生新 semantic_rev。
- [ ] **R07-A02** 在每个持久提交阶段注入失败，恢复后得到已提交一致状态或明确冲突，没有半数事实被隐藏。
- [ ] **R07-A03** 保留既有跨 store 五个中断阶段测试；新单 store 多对象操作不破坏它们。
- [ ] **R07-A04** 恢复期间用户手改任意目标时不覆盖；journal 与现场保留可诊断。
- [ ] **R07-A05** v1/v2 旧 pending journal 可恢复；新 journal 中越界/符号链接/重复目标/哈希不符被拒绝。
- [ ] **R07-A06** 旧记忆首次纳入历史只记录当前快照与 coverage 起点，不能生成虚构的过去 body。
- [ ] **R07-A07** 从规范 Markdown+manifest/证据重建 SQLite 后，当前状态与历史版本不丢失。

不做：不做 CRDT/网络复制、不把 SQLite 改为唯一知识源、不宣称多文件普通 rename 已是原子事务。
回滚：在副本上先升级并验证恢复；旧 writer 不可处理新 journal 时禁止混写。回退使用备份或兼容导出，绝不删未决 journal 强行启动。
参考：[S11](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/relations.rs)；[S12](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-migration.md)；[E02](https://github.com/getzep/graphiti)；[E09](https://github.com/akitaonrails/ai-memory/blob/main/docs/temporal.md)。

### R08 — system-time 历史查询与保守代码适用性
优先级：P2；阶段：M2；依赖：R07。
目标：能回答“这个时刻库里记录了什么”，但不承诺世界真实时间或历史排序复现。
落点：`src/search.rs`、`src/api.rs`、`src/schema.rs`、`src/main.rs`、`src/mcp_tools.json`、`src/context.rs`、`src/eval.rs`。
实施：
1. 新增 history/show --revision/search --as-of 的共享 API。历史窗口统一 [system_from,system_until)，timestamp 解析 RFC3339；仅日期输入固定解释为 UTC 00:00 并回显。所有时间由可注入 clock 产生。
2. 旧 created 与未知 supersede 时间不强行回填；超出 coverage 返回 partial_history/unknown，而非空结果冒充不存在。
3. 第一版历史检索仅使用版本化 lexical 数据；禁用现时 vector、现时图扩展、access bump 和当前视图回退。以后没有 revision-scoped 证据就不得重新开启这些 lanes。
4. 结果包含历史版本的 status/expiry/provenance。旧 expires 仍是 inclusive 日期，历史到期状态依据查询明确偏移的日历日计算并回显；不能拿今天的 is_expired 筛掉过去有效条目。
5. 新增 observed_commit 与 applies_to 是不同字段。默认只承认 repo-wide、explicit exact commit/tree、明确 related-path digest；不要推断一个祖先 commit 的事实永远适用于后代。branch 只展示，unknown applicability 明示。
6. 使用当前索引词频排序历史内容可能受后来文档影响；第一版只保证内容版本正确，不承诺当时排名完全重现。严格历史重放作为独立评测模式使用冻结语料。

验收：
- [ ] **R08-A01** 在 T1 保存 PostgreSQL，T2 明确替代为 SQLite；T1≤T<T2 只看到当时版本。
- [ ] **R08-A02** 当前已经过期的事实在过去有效时间仍可查；跨日期/偏移边界规则一致且输出解释。
- [ ] **R08-A03** 没有历史覆盖时返回 partial/unknown；不将 recorded_at=今天的旧文件伪装成其 created 那天的完整快照。
- [ ] **R08-A04** 加入未来文档、当前 supersedes 边、当前向量后，不会泄漏未来正文/关系进历史结果。
- [ ] **R08-A05** 历史只读查询不改 usage、expiry、session 去重或 canonical memory。
- [ ] **R08-A06** 切到旧 commit、相同 commit 的脏工作区、两分支同名文件：适用性不同或 unknown，不自动套用最新结论。

不做：不自动 LLM 推断现实生效时间、不默认实现 --as-of <commit> 与时间同义、不做通用 Git 分支知识合并。
回滚：关闭历史选项不影响当前搜索；历史索引可重建，历史 Markdown/manifest 不是缓存。
参考：[S05](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/context.rs)；[S08](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/search.rs)；[S12](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-migration.md)；[E02](https://github.com/getzep/graphiti)；[E09](https://github.com/akitaonrails/ai-memory/blob/main/docs/temporal.md)。

### R09 — Reconciliation 与 Proposal Ledger
优先级：P1；阶段：M2；依赖：R07, R04。
目标：解释“支持、补充、不同上下文、冲突和替代”，并以可审核操作执行，而不是重新引入智能覆盖。
落点：`新增 src/reconcile.rs`、`新增 src/proposals.rs`、`src/api.rs`、`src/ingest.rs`、`src/relations.rs`、`src/mcp_tools.json`。
实施：
1. 定义枚举 CREATE/SKIP/SUPPORT/REFINE/CONTEXTUALIZE/CONTRADICT/SUPERSEDE 和带 evidence、reason、targets、expected_revisions 的 Decision；策略与执行分离，执行复用 R07。
2. 默认自动处理确定性的 exact replay/CREATE/独立来源 SUPPORT；相似度只召回候选。模型生成的 refines/caused_by/contradicts/supersedes/摘要重写均为提案。
3. fact_key 需同时匹配主体、项目/环境、属性和单值/多值约束；同 key 不同值也可能是环境差异或低可信推测，不能直接 supersede。明确目标+授权+CAS 才可应用替代。
4. 提案持久保存 generated/pending/approved/applied/rejected/stale 状态与变更摘要哈希。批准时重新读取所有目标 revision；不一致标 stale，不能静默重基。
5. 默认 agent 可以提出提案，但不能自行批准有语义破坏性的提案。审批走明确的人类 CLI/受信管理入口；tool annotation 不是授权系统。
6. 批准过程只执行结构化领域操作，不运行提案里的 shell。审批重试幂等；undo 以有前置条件的补偿修订实现，存在后续依赖时报告冲突。
7. 有疑似矛盾的新事实仍可保守追加并显示未核实/冲突候选，不因无人审批而阻塞正常记忆写入。

验收：
- [ ] **R09-A01** 相同主题互补事实为 CREATE/REFINE 候选，不自动失效；多值属性不产生假冲突。
- [ ] **R09-A02** 同属性不同环境被识别为不同上下文；同范围单值冲突有证据说明但不自动选择赢家。
- [ ] **R09-A03** 不同来源同事实 SUPPORT 追加来源；同源转述不倍增证据。
- [ ] **R09-A04** 提案生成后目标被另一个 agent 修改，批准返回 stale 且无半应用。
- [ ] **R09-A05** 重复批准/applied 重放幂等；对不具审批授权的 MCP 调用拒绝，其他工具正常。
- [ ] **R09-A06** 语义修改全部经过 R07 恢复/历史链；undo 不覆盖后续外部修改或删除证据。

不做：不强制 LLM judge、不按 agent 名称给真伪排名、不根据高 strength 自动信任。
回滚：关闭自动建议即可回到保守写入；提案文件保留。已应用动作使用补偿修订而非覆写旧历史。
参考：[S03](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs)；[S11](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/relations.rs)；[E08](https://github.com/tigerless-labs/agent-memory)；[E10](https://github.com/ourmem/omem)；[E11](https://github.com/yantrikos/yantrikdb)；[E12](https://modelcontextprotocol.io/specification/2024-11-05/server/tools)。

### R10 — 可重复维护与热度/有效性分离
优先级：P2；阶段：M2；依赖：R07, R01。
目标：多个 agent 共用 store 时，维护次数不再无意决定知识寿命。
落点：`src/api.rs`、`src/schema.rs`、`src/store.rs`、`src/context.rs`、`src/main.rs`。
实施：
1. 保留 per_run 兼容模式；新增显式 per_day 模式，启用时初始化 last_maintained_at，不根据旧 created 一次扣尽所有历史天数。
2. 按固定 clock 计算 elapsed days 并记录已应用周期，多次同日维护幂等；时钟回退时不产生反向奖金或巨额惩罚。
3. 区分 usage_strength、semantic validity、verification；保留旧字段投影，衰减不能自动改真假。明确 superseded/expired/deprecated/archive 的交互。
4. core candidate 只给候选，提升 core 仍需提案/明确确认；低频但关键记忆可 pinned，不因少召回自动丢失。
5. 维护 dry-run 不改 canonical records；状态/归档变化经 R07；兼容原有 archive 位置和 explicit history 查询。

验收：
- [ ] **R10-A01** 同一天运行一次/十次维护结果相同（per_day），per_run 保留旧契约。
- [ ] **R10-A02** 启用 per_day 的旧库不因历史年龄被一次归档；时钟回退、跨日边界可重复测试。
- [ ] **R10-A03** 被频繁注入只提升热度，不提升 verification 或解除 superseded。
- [ ] **R10-A04** pinned 关键低频知识不因热度归档，但明确 supersede/expiry 仍按公开策略处理。
- [ ] **R10-A05** dry-run 不改文件/历史；实际状态变化只有一份对应 revision。

不做：不复制复杂 Weibull/强化学习策略、不启用 cron、不自动 core promotion。
回滚：配置回到 per_run 不倒写历史计量；字段保留兼容，任何语义回退需补偿修订。
参考：[S03](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/api.rs)；[E10](https://github.com/ourmem/omem)；[E11](https://github.com/yantrikos/yantrikdb)。

### R11 — 有界 sleep 与宿主辅助提案
优先级：P2；阶段：M3；依赖：R09, R06。
目标：把 Gemini 的睡眠提炼转成显式、可审查、默认不修改事实的维护工作流。
落点：`新增 src/sleep.rs`、`src/ingest.rs`、`src/proposals.rs`、`src/main.rs`、`src/mcp_tools.json`。
实施：
1. sleep 明确调用；输入只扫描 since_cursor 之后的有限变更/检查点，设置数量、字节、输出 token 和提案数量上限。已有启发式/host/外部模型模式复用而非另建 client。
2. 规则模式能离线生成确定性重复/引用缺失等报告；语义模式输出 proposal。自动修复仅限满足证明性前置条件的派生索引或已有关系镜像修复，不自动猜 caused_by。
3. host 模式先 export 结构化 request，宿主返回 proposals 再 import；不从不可信记忆中取 shell 指令执行。可选外部模型遵守全局端点和显式授权。
4. 生成报告记录输入 refs/revisions、输出和 cursor；失败或输出验证失败不推进 cursor，部分成功可安全重放。
5. 禁止默认保留全量私人 transcript；报告只含必要的脱敏内容，不能把检测到的密钥再写进 audit log。
6. 全程不自动安装定时器，不把每个 Stop 变成长时维护；请求到来时仍优先正常检索。

验收：
- [ ] **R11-A01** 关闭所有模型仍能执行规则 sleep/report；不发生网络请求。
- [ ] **R11-A02** 相同 input snapshot 重跑不重复生成等价提案；不同 revision 会生成新候选或 stale。
- [ ] **R11-A03** 模型失败、非法输出、半批提案失败时 cursor 不丢未处理输入。
- [ ] **R11-A04** 模型声称高置信度 caused_by/supersedes 仍待审，不直接改 Markdown。
- [ ] **R11-A05** 输入/输出/提案数超上限时明确 partial 与下一游标，不无限循环。
- [ ] **R11-A06** 报告含来源但不含凭据；未授权 sleep 不能改 core、执行命令或启用 cron。

不做：不把 Dream Report 当事实、不引入后台 daemon、不要求每次写入跑模型。
回滚：关闭 sleep 保留基础能力；报告与待审提案可以归档，已应用变更仍按历史管理。
参考：[E08](https://github.com/tigerless-labs/agent-memory)；[E10](https://github.com/ourmem/omem)；[S10](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/main.rs)。

### R12 — 可追溯的派生知识页
优先级：P2；阶段：M3；依赖：R11, R08。
目标：借鉴 Hindsight/Letta 的归纳层，但不让旧摘要成为无法撤销的新事实。
落点：`新增 src/views.rs`、`src/context.rs`、`src/search.rs`、`src/proposals.rs`、`docs/interface.md`。
实施：
1. 生成 views/architecture.md、pitfalls.md、testing.md 等可选视图，记录 derived_from 的 MemoryRef+revision、生成方式、覆盖范围与 known gaps。
2. 源 revision 变动/supersede/失效时标 stale，并退出默认注入；重新生成需当前快照，不以旧视图替代原始证据。
3. 不覆盖人工 core.md。视图与原始事实在检索和上下文中标类型，避免同一内容占两次预算或被算两份支持证据。
4. 经验转规程仅输出候选，需成功证据、适用条件、失败边界和审批；不依据一次 agent 自述产生自动 skill。
5. 派生文件可以重建；人工编辑视图必须转为独立知识或显式 unmanaged，不覆盖用户手改。

验收：
- [ ] **R12-A01** 每条关键结论可追溯到具体 source revision；不存在的引用使生成失败或明确 partial。
- [ ] **R12-A02** 源被替代后视图不再作为当前确定知识注入；旧视图仍可审计。
- [ ] **R12-A03** 原始事实与派生摘要不构成两个独立证据；相同上下文不双重占预算。
- [ ] **R12-A04** 人工 core 不被自动改；视图外部手改触发冲突而非静默覆盖。
- [ ] **R12-A05** 无模型情况下仍能按模板生成索引式视图，不伪称完成语义综合。

不做：不做自动技能发布、不运行生成规程、不升级成 agent framework。
回滚：禁用 views 即恢复事实级召回；保留来源版本与人工内容，派生文件可在明确检查后重建。
参考：[E03](https://github.com/letta-ai/letta)；[E06](https://github.com/vectorize-io/hindsight)。

### R13 — 原生快照、迁移与安全恢复
优先级：P1；阶段：M3；依赖：R07, R06, R09。
目标：继承 Memvid 的便携与校验，不把 archive Markdown 换成私有二进制真源。
落点：`新增 src/snapshot.rs`、`src/store.rs`、`src/relations.rs`、`src/main.rs`、`docs/rust-migration.md`。
实施：
1. 快照包含 canonical Markdown、history、store manifest、证据/检查点/提案元数据和 schema/软件版本；config 采用安全白名单。默认排除原始 transcript、密钥、模型、可重建 SQLite/WAL 与宿主设置。
2. 先持锁检查/恢复 pending 操作；未决跨 store coordinator 不齐全时拒绝“可迁移完整快照”。不得搬迁一个参与者后宣称仍能依赖旧 global root 恢复。
3. 优先实现标准目录快照+SHA-256 manifest；可选 tar/zip 使用经过审查的依赖，不能 shell 拼接不可信路径。容器格式不改变知识真源。
4. 恢复默认新空目录，先验证条目数量/大小上限、路径规范化、重复路径、symlink/hardlink、哈希与 schema，再发布目录；失败不留下貌似完整 store。
5. 保留 store_id 的 restore 与创建新独立副本的 fork 分开；同时在线的相同身份副本必须可检测。首版不静默合并两个 store。
6. 验证恢复后重建缓存；私有本机切换记录里的备份路径与脚本不作为公开实现依赖。

验收：
- [ ] **R13-A01** 备份→空目录恢复→重建 SQLite 后 facts/provenance/history/checkpoints/proposals 数量与哈希符合 manifest。
- [ ] **R13-A02** 路径 ../、绝对路径、符号/硬链接、重复条目、超大解压、损坏哈希均被拒绝。
- [ ] **R13-A03** 恢复失败不覆盖原 store；真实用户数据不作为默认测试输入。
- [ ] **R13-A04** 存在未完成跨 store 操作时拒绝不完整迁移，不盲删 coordinator decision。
- [ ] **R13-A05** 默认包中没有凭据/API selectors/敏感 transcript；脱敏策略及被排除项在 manifest 可见。
- [ ] **R13-A06** restore 保身份与 fork 新身份行为各自明确，旧裸 ID 引用不会静默指向另一独立库。

不做：不做云同步、不把压缩包当在线索引、不自动恢复真实库。
回滚：恢复默认新目录；回退即保留旧目录不切换。新 schema writer 切换必须停旧 writer，并保留可验证的迁移前快照。
参考：[S11](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/relations.rs)；[S12](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-migration.md)；[S13](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-local-cutover.md)；[E07](https://github.com/memvid/memvid)。

### R14 — 实测驱动的性能与检索评测
优先级：P1；阶段：横贯全程；依赖：R00。
目标：Rust 重构已完成，现在用证据优化，不能继续把“Rust 化”本身当性能项目。
落点：`src/search.rs`、`src/models.rs`、`src/vectors.rs`、`src/eval.rs`、`新增 benches 或 tests/perf`、`assets/eval/`、`.github/workflows/ci.yml`。
实施：
1. 在 R00 后立即建立无模型 release 基线：进程启动、冷建索引、热查询、单文件变更、文件路径注入、pending recovery。合成 100/1k/10k 条，记录硬件、build profile、模型开关和数据哈希。
2. 分解耗时为文件枚举/stat、SQLite 同步、候选读取/反序列化、图扩展、模型 hash/session 构造、context 渲染。审阅当前 sync_store 每次枚举文件和 fingerprint 读取模型的成本，但不先断言是主瓶颈。
3. 保持 FTS/BM25/RRF/typed graph/reranker 已有功能；新增 lane ablation 与噪声/无关注入率，不再开发重复 graph-hopping。
4. 语义正确性是硬门禁。若优化需要弱化内容哈希、跳过外部编辑检查或不恢复 pending，方案不合格。可能引入进程内模型 handle 缓存或更便宜同步策略，但须先说明一致性和失效。
5. 评测分三类：确定性回归、完整公开语料的 retrieval、固定模型/预算的 end-to-end。报告 requested/effective pipeline，意外 fallback 不能继续冒名通过。
6. 重复测量报告 p50/p95 与样本数。相对性能回退阈值在取得稳定基线后设定；暂不硬编码“Rust 一定<10ms”或随机 CI 机器绝对门限。

验收：
- [ ] **R14-A01** 发布可复跑命令、原始 CSV/JSON、数据哈希和环境，所有性能数值与明确提交绑定。
- [ ] **R14-A02** 旧三个 recall 门禁不降低；benchmark fixture 不作为 training/tuning 后唯一测试集。
- [ ] **R14-A03** 词法→向量→图→rerank 消融明确每一路实际启用状态，不混用 mock 和真实推理结论。
- [ ] **R14-A04** 外部同大小正文修改、删除、rename、并发读写、缓存损坏、pending recovery 在优化前后行为一致。
- [ ] **R14-A05** 跨 agent 端到端使用无记忆/本版/改进版相同任务与预算，记录错误事实使用率和接手续作结果。
- [ ] **R14-A06** 索引冷建变慢与热查询收益分别报告，不用某一项平均值笼统宣称全面提速。

不做：不再提 Rust probe 重写、不无证据引入 ANN/图数据库/常驻 worker、不使用未经授权真实会话评测。
回滚：所有优化单独可回退；固定评测语料与原始结果保留，回退不影响 canonical 数据格式。
参考：[S06](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/vectors.rs)；[S07](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/models.rs)；[S08](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/src/search.rs)；[S13](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-local-cutover.md)；[E05](https://github.com/thedotmack/claude-mem)；[E14](https://github.com/xiaowu0162/LongMemEval)。

### R15 — 发布候选收口与执行交接
优先级：P1；阶段：M4；依赖：R01, R02, R03, R04, R05, R06, R07, R08, R09, R10, R11, R12, R13, R14。
目标：按明确能力边界验收下一代原生版本，不能以“功能代码已经写了”替代可用性。
落点：`.github/workflows/ci.yml`、`Cargo.toml`、`Cargo.lock`、`README.md`、`README.zh.md`、`docs/rust-migration.md`、`docs/interface.md`、`assets/templates/`、`docs/plans/native-evolution/`。
实施：
1. 冻结 CLI/MCP 和持久 schema 契约；所有新能力有机器可读 status/warnings，旧 alias 按兼容承诺保留。pub Rust API 的新增字段可能破坏 struct literal；使用新请求类型/builder 或明确 crate semver 变更，不假称增加字段总是兼容。
2. Linux/macOS CI 验证 default/all-features 编译、测试、包内资源及原生工具；本地 ONNX 资产可选独立作业，缺失写 NOT_RUN，而非 pass。
3. 主机验收按三个层次记录：纯协议 fixture、真实配置 runner、真实应用/模型会话。没有实际做哪层就不宣称通过；默认不更改已有聊天和宿主设置。
4. 检查源码包和二进制包、checksum、依赖/许可证、MSRV、ORT 动态库要求。package version≠已发布 release；打 tag/push/publish 需另行用户授权。
5. 更新 README/迁移指南/执行日志：已完成任务、未完成边界、迁移前置、恢复方法、实际测试结果。不宣称零 bug、任意模型支持或任意 Git 时间旅行。

验收：
- [ ] **R15-A01** 全部任务验收条目有 pass/fail/not_run/blocked；没有未运行却写 pass 的条目。
- [ ] **R15-A02** 源码打包白名单继续有效；二进制不依赖 Python 内核，开发 smoke 脚本可以保留。
- [ ] **R15-A03** 从旧 1.0.0 副本升级→复跑→快照恢复完整演练；所有 pending 操作有明确处置。
- [ ] **R15-A04** 四当前宿主的协议合同通过，真实宿主/真实模型证据单列；不重新宣称 Hermes support。
- [ ] **R15-A05** 版本与 schema 变更、回滚限制、尚未实现选项在中英文文档一致；未经授权没有发布或安装操作。

不做：不自动 push/tag/publish，不修改用户真实记忆，不删历史验收记录。
回滚：发布前保留旧二进制和迁移前已验证快照；入口切换单独授权；新数据不能直接交给旧 writer。
参考：[S01](https://github.com/Furinelle/Mnemosyne/commit/d05f02853f42771ae9ca04d78b79739d01f1a76c)；[S02](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/Cargo.toml)；[S13](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/docs/rust-local-cutover.md)；[S14](https://github.com/Furinelle/Mnemosyne/blob/d05f02853f42771ae9ca04d78b79739d01f1a76c/.github/workflows/ci.yml)；[S15](https://github.com/Furinelle/Mnemosyne/actions/runs/35441992545)。

## 7. 标准执行与测试门禁

先确认工作区 AGENTS.md 和现有 rust-toolchain.toml。本轮生成计划没有运行以下命令，它们是 Codex 实施时的门禁。

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo run --locked -- eval run --min-recall 0.95
cargo run --locked -- eval run --pipeline full --min-recall 0.95
cargo run --locked -- eval run --longmemeval --pipeline full --min-recall 0.95
cargo build --locked --release --features onnx
```

这些命令须由隔离测试包装器执行：保存工具链定位，设置临时 HOME/MNEMOSYNE_HOME/CARGO_TARGET_DIR，将运行态 CWD 放在临时项目。Cargo 的 manifest 仍定位实际工作区。运行 native-only 子进程保持 env_clear；模型/网络测试用本地假服务并显式白名单参数。PATH 为空的测试通过绝对二进制路径运行，而不是依赖 cargo 位于空 PATH。

模型路径存在不代表资产可信或与当前 profile 相容。真实 ONNX 测试需另行明确提供 model/vocab/ORT 及 hash，记录对应平台和 feature。HTTP mock 只证明协议和错误处理，不证明真实供应商质量。无资产时记录 NOT_RUN；不得静默下载。

每个 PR 的完成条件：新增回归先红后绿；原有 gates 不降低；检查 stdout/stderr 契约；更新 docs 与 EXECUTION_LOG；review diff 只含范围内改动；说明副作用/数据迁移/回滚。基础环境错误不能改成 skip 后称全绿。

## 8. Codex 工作方式

默认先完成 R00，再完成 R01/R02/R03，最后跑 R14 基线。若实际最新 HEAD 已修复某问题，保留证明性回归并标 verified-already-implemented，不能再次实现或故意引入失败。之后按 depends_on 逐个选择任务，不能把整张路线图放一个巨大提交。

允许多 agent 在不同文件边界承担测试/文档/独立修补，但必须以同一冻结 schema 和接口为准；每次 merge 后复跑综合门禁。同一 store/迁移/事务代码安排一个集成人，不能产生两种 history/receipt 设计。

上下文不足时，把 HEAD、dirty 文件、完成条目、实际命令、失败原因和下一个任务写入执行日志；不把“已计划”写为“已实现”。如果测试不能跑，提交可审阅的实现与明确限制，不擅自降低门禁。此条是会话交接要求，不是授权异步后台工作。

## 9. 与前文建议的最终取舍

采纳 Gemini 的 sleep/report/proposal、时态、模型一致性和预算方向，但按 Rust 当前实现重新划分：fingerprint/整包预算/graph hopping/关系恢复已存在，只做增量。自动推断 caused_by 不是机械只读操作；不能自动应用。单文件只作快照，Markdown archive 保留。Rust 已是内核，因此彻底取消“以后 Rust 化”任务。

保留前文的 provenance、reconciliation、checkpoint、history、views，但删去 Python 模块路径、portalocker 依赖承诺、Hermes Python 适配器与旧 0.9→1.0 版本序列。新的第一目标不是堆更多检索模块，而是：**来源不丢、坏缓存可修、协议可验、换 agent 能续作**。

详细原始来源见 REFERENCES.md；基线审阅结论见 BASELINE.md；机器任务和验收编号见 BACKLOG.json / ACCEPTANCE.md。
