# Mnemosyne

[![CI](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml/badge.svg)](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml)

**Mnemosyne 是用 Rust 实现的本地优先记忆内核。** Codex、Claude Code、Grok Build
和 Antigravity 通过原生 CLI、MCP 与生命周期事件，共享可直接阅读的 Markdown 记忆。
记忆操作无需 Python、常驻后台服务、LLM 账号或独立数据库服务器。

[English](README.md) · [迁移与兼容说明](docs/rust-migration.md)

## v2.0.1

v2.0.1 新增显式来源、任务检查点、语义修订历史、按系统时间查询、保守 Git
适用性、审批提案、按日维护、有界离线 sleep、派生来源索引及可校验目录快照。
具体契约见 [接口文档](docs/interface.md)，实际验证见
[执行记录](docs/plans/native-evolution/EXECUTION_LOG.md)。

这是 Rust crate 的破坏性发布：公开结构体新增字段可能破坏下游字面量构造。CLI
兼容不等于 crate 源码兼容；八个 MCP 工具名称与原生 aliases 保持兼容。

2.0.1 修复升级后 ingest／Stop／蒸馏写入、结构化冲突分类及修订后来源追加，
并让 sleep 在单页字节预算内返回可继续处理的分页。

## 下载

- [macOS ARM64](https://github.com/Furinelle/Mnemosyne/releases/download/v2.0.1/mnemosyne-2.0.1-aarch64-apple-darwin.tar.gz)
- [Linux x86_64](https://github.com/Furinelle/Mnemosyne/releases/download/v2.0.1/mnemosyne-2.0.1-x86_64-unknown-linux-gnu.tar.gz)
- [源码包](https://github.com/Furinelle/Mnemosyne/releases/download/v2.0.1/mnemosyne-2.0.1-source.tar.gz)
- [SHA256SUMS](https://github.com/Furinelle/Mnemosyne/releases/download/v2.0.1/SHA256SUMS)

安装前请用 `SHA256SUMS` 校验下载文件。发布二进制包含可执行文件及发布文档；ONNX
Runtime、模型文件和 `vocab.txt` 是外部依赖，未被捆绑。

## 构建与安装

```sh
git clone https://github.com/Furinelle/Mnemosyne
cd Mnemosyne
cargo build --locked --release --features onnx
mkdir -p "$HOME/.local/bin"
cp target/release/mnemosyne "$HOME/.local/bin/mnemosyne"
export PATH="$HOME/.local/bin:$PATH"
mnemosyne --help
```

`onnx` feature 提供可选的本地模型推理能力。实际使用还需要兼容的 ONNX Runtime
动态库、模型文件及其 `vocab.txt`。把 macOS 的 `libonnxruntime.dylib` 或 Linux 的
`libonnxruntime.so` 放在可执行文件旁，或设置 `ORT_DYLIB_PATH`。程序不会自动下载模型。
基础记忆操作不要求动态库；无需本地推理时，可省略构建参数 `--features onnx`。

构建不会自动切换已有 Python 安装或宿主设置。宿主配置宜使用原生可执行文件的绝对路径，
避免 `PATH` 仍指向旧入口。

## 升级已有 store

v2.0 使用 schema 2 / writer protocol 3。先停止所有旧 writer，再备份并验证全部 canonical
store data，另行保存宿主配置。先预览，再提交升级：

```sh
# 在当前主机存在的每个 scope 上执行。
mnemosyne store-upgrade --scope global
mnemosyne store-upgrade --scope project

# 仅在旧 writer 全部停止且完整备份已验证后执行。
mnemosyne store-upgrade --scope global --commit
mnemosyne store-upgrade --scope project --commit
```

`snapshot DESTINATION` 与 `restore SOURCE TARGET` 会校验 canonical store 的 manifest 并重建
派生搜索缓存；请先恢复到独立的空目录以验证备份。snapshot 不包含宿主配置。回滚时恢复
完整的升级前 canonical store data、还原另存的宿主配置，再切回兼容的旧 writer。不要删除
`store.json`，也不要让旧 writer 操作已升级 store 来代替回滚。

## 快速开始

```sh
cd /path/to/your/project
mnemosyne init --agent codex
mnemosyne write --type codebase --importance 70 \
  --title "认证服务" --content "认证 token 在 15 分钟后失效。"
mnemosyne search "认证 token" --format json
mnemosyne prep "排查登录回调"
```

全局偏好保存在 `~/.mnemosyne/`，可通过 `MNEMOSYNE_HOME` 改位置；项目知识保存在
项目内 `.mnemosyne/`。`core.md` 是小型常驻核心，`working/*.md` 与 `archive/` 保存
独立记忆。Markdown 是真源，SQLite 搜索和向量缓存可以重建：

```sh
mnemosyne reindex --scope all
mnemosyne embed-backfill --scope all  # 仅在已配置并启用 embedding 时使用
```

## 接入四个宿主

| 宿主 | 接入方式 | 初始化与配置建议 |
|---|---|---|
| Codex | 会话/prompt/Stop hooks、项目指令、CLI 或 MCP | `mnemosyne init --agent codex`；`mnemosyne install codex` |
| Claude Code | SessionStart / UserPromptSubmit / PreToolUse / Stop hooks | `mnemosyne init --agent claude-code`；`mnemosyne install claude-code` |
| Grok Build | Claude 兼容 hooks、项目指令、Grok JSONL 蒸馏 | `mnemosyne init --agent grok`；`mnemosyne install grok` |
| Antigravity | MCP，通过显式项目路径选择记忆库 | `mnemosyne init --agent antigravity`；`mnemosyne install antigravity` |

`init` 创建项目文件并保留已有指令文件。`install` **只输出配置建议**，不改宿主设置，
也不代表宿主已经加载配置。模板位于 `assets/templates/`。

启动原生 MCP 服务：

```sh
mnemosyne mcp serve
```

工具包括 `mnemosyne_search`、`mnemosyne_write`、`mnemosyne_read_core`、
`mnemosyne_show`、`mnemosyne_link`、`mnemosyne_graph`、`mnemosyne_maintain`、
`mnemosyne_prep_context`。Antigravity 等宿主可能从项目外启动服务，此时工具参数要传
绝对路径 `project_path`。没有项目上下文的项目请求会明确报错，不会写到进程根目录。
项目配置中的 `mcp.expose_global`、`mcp.expose_project` 控制对应作用域是否对 MCP 开放。

CLI 与 hooks 使用四个中立事件，JSON 从 stdin 输入：

| 事件 | 输入 |
|---|---|
| `session_start` | `{}` |
| `turn_start` | `{"prompt":"..."}` |
| `file_touch` | `{"files":["src/auth.rs"]}` |
| `session_end` | `{"text":"..."}` 或 `{"transcript":{"path":"...","format":"auto"}}` |

```sh
printf '%s\n' '{"prompt":"排查登录回调及认证逻辑"}' |
  mnemosyne inject --event turn_start --session my-session --format json
```

`mnemosyne hook EVENT` 输出 Claude 兼容的事件封装，不会发出工具权限放行决定。
`inject --fail-safe` 出错时将诊断写到 stderr、stdout 保持空白，并正常退出。
注入预算计入整份交付文本，属于 **estimated 估算**，不是任意模型的精确 token 上限。

## 已有能力与边界

- SQLite FTS5、中文等 CJK 文本检索；可选向量、RRF 融合、关系扩展与 cross-encoder 重排。
- 来源、记录日期、有效状态与 evidence。默认检索过滤过期及已替代记录；记录日期不等于核查日期。
- 保守写入：词汇相似不能自动授权丢弃或替代有变化的事实。显式关系与整合仍可用，整合先预览候选。
- 按 UTC 日幂等执行生命周期维护、归档与 core 候选；legacy maintenance 调用归一到
  同一日维护记账，候选不会自动改写 `core.md`。
- 可选蒸馏支持 Claude、Codex、Grok、role JSONL 和文本。保留消息角色边界，默认不把推理或工具内容作为证据。
  `distill` 不带 `--commit` 时只预览；会话自动蒸馏由 `[distill].enabled` 控制，默认关闭。
- 原子 Markdown 写入、文件锁与可恢复关系变更，支持经协调的跨 store 关系写入。
  移动记忆库或改变全局协调器位置前，须先恢复未完成操作。

网络端点、凭据环境变量选择器、本地模型路径只信任全局配置。LLM 抽取与模型推理都是可选项。
详细格式见 [接口约定](docs/interface.md)、[findings 交换](docs/handoff-format.md)
与 [适配器映射](docs/adapters.md)。

运行时已转为原生实现，旧 Python 包/import API 和 Hermes Python MemoryProvider 退役。
保留 `codex-prep`、`codex-ingest` CLI 别名及 `mnemosyne_codex_prep` MCP 别名。
已有部署切换前请阅读 [迁移说明](docs/rust-migration.md)。

## 开发与验证

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo run --locked -- eval run --min-recall 0.95
cargo run --locked -- eval run --pipeline full --min-recall 0.95
cargo run --locked -- eval run --longmemeval --pipeline full --min-recall 0.95
```

模板与内置评测数据分别位于 `assets/templates/`、`assets/eval/`。
`tests/native_only.rs` 在隔离 HOME、空 PATH 下运行原生二进制，验证事件协议、transcript
重放和 MCP 项目隔离；这类模拟客户端验收不能替代真实宿主验证。

`tests/native_*_smoke.py` 是可选的 **开发验证脚本**，仅使用 Python 标准库驱动原生二进制；
它们不属于安装后的记忆内核，也不构成运行时依赖。模型 smoke test 需要另行提供本地模型；
假服务协议测试不能证明真实模型质量。内置 LongMemEval 样本也不等于完整公开基准。

v2.0.1 测试套件包含 159 项测试；Linux 与 macOS CI 运行发布检查。本地宿主 hook 检查覆盖
协议行为，不等于四个真实模型对话验收；可选 ONNX 模型验收取决于本机环境。此版本不作
笼统的性能提升承诺。

[验证记录](docs/rust-validation.md) · [本机切换记录](docs/rust-local-cutover.md)
· [更新日志](CHANGELOG.md)。这些记录只说明各自版本、时间与验证范围，不代表后续构建及所有宿主均已通过。

许可证：MIT。
