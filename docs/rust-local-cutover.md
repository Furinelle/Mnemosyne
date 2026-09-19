# 本机 Rust 切换与功能验收（2026-09-19）

本文件记录两个先后完成的本机验证阶段，不是后续发布版本的统一验收证明。
第一阶段切换 CLI/hooks/MCP，暂留 Python；第二阶段见文末 Native-only completion，
已退役 Python 内核。各阶段的测试数、行为边界与发布状态只适用于当时的快照。

## 第一阶段：实际切换

- CLI：`~/.local/bin/mnemosyne` 现指向原生可执行文件。
- 原生版本目录包含可执行文件与 ONNX Runtime 动态库。
- 本阶段二进制 SHA-256：`0aacc30e03b70850c7cb2b9c19c3e48f08115eba0b3333a2ffcb43977325a7b9`，不代表后续发布产物。
- Codex：更新 `~/.codex/hooks.json` 的三个 Mnemosyne hook 命令。
- Claude：更新 `~/.claude/settings.json` 的四个 Mnemosyne hook 命令。
- Grok：通过 `grok inspect --json` 确認其继承的四个 hooks 已解析到原生命令。
- Antigravity：全局与插件两份 MCP 配置均切换；界面刷新后，两路驻留进程实际为原生二进制，两路均显示 8 个工具可用。
- 全局使用规则、Antigravity 插件规则/技能和当前项目 AGENTS 的 CLI 示例改为原生入口。

JSON 配置结构对比确认：只改变目标 hook/MCP 的 command 值，未改变其他插件、权限、模型或凭据配置。

Antigravity 的 MCP 进程实际从 `/` 启动，原配置无法提供项目上下文。现每个
MCP 工具支持可选 `project_path`，项目操作应传入当前项目绝对路径。
未提供项目上下文时，不会隐式向 `/.mnemosyne` 写入。全局记忆仍可独立读取。

## 实测与证据范围

| 功能 | 验收结果 |
| --- | --- |
| 原生测试 | 32 项库测试 + 8 项 CLI 集成测试通过 |
| 原 Python 回归 | 310 项测试 + 6 项 subtests 通过；不等价于宣称全部 Rust 行为相同 |
| 格式与静态检查 | rustfmt、Clippy all-features、Ruff 通过 |
| 检索门槛 | 固定 50 题 BM25 与 FTS/fusion 均 recall@5=1.000；LongMemEval 内置 2 题样本通过 |
| 自定义评测 | 修复 `--longmemeval` 忽略 `--corpus`；补齐 convert，兼容 Python 转换器无 type 的 seed 格式 |
| 读写与并发 | 精确重放、近似但不同事实、有效期变化、8 进程并发重放、旧格式读取与 MEMORY.md 目录索引同步通过 |
| 过滤与图 | 默认/归档/过期/type/superseded、短中文+英文混合检索、跨 scope 同 ID 的 legacy link 对照通过 |
| 索引 | 增量同步、外部手改、删除、稳定读校验、缓存路径保护与关系恢复锁通过 |
| Hooks | 真实配置的 SessionStart、UserPromptSubmit、PreToolUse、Stop 命令返回正确 JSON 或空输出；退出码全部 0 |
| Stop 差异测试 | 临时库中 Claude/Grok/Codex × heuristic/host、重复与追加 transcript、角色/source 保留通过 |
| Antigravity MCP | UI 与驻留进程核验；按两份真实配置从 `/` 启动服务器，初始化、tools/list、真实库 core/search、维护预览、prep 均通过 |
| SSE | 实际握手、ping、非法 UTF-8 不终止服务通过 |
| Hermes | 薄宿主适配器调用原生内核的独立验收通过；本机没有因此安装/启用 Hermes |
| HTTP 模型后端 | 本地模拟 OpenAI-compatible 服务验证 embedding 批次顺序、向量检索、LLM 蒸馏；未消费真实外部 API |
| 真实 ONNX | 实际预训练模型完成 embedding 回填、向量召回、缓存复用与 cross-encoder 重排；不是只做编译检查 |

本次没有另开 Codex/Claude/Grok 模型会话代替用户发送任务；hooks 的证据是
当前配置解析与真实命令执行。Antigravity 的证据还包括正在运行的应用进程和 UI，
但未向用户已有聊天发送测试消息。

## 实测中发现并修复

1. Rust 原先在 FTS 命中时丢弃短中文召回，现合并两路候选。
2. Rust 原先每次搜索全量重建索引，现基于 mtime/size 做增量同步；同步期间遵守 store 锁和恢复协议。
3. legacy bare link 同 ID 跨库曾扩展多个目标，现保持原传入 store 顺序的首个目标语义。
4. 注入访问加分现封顶 100；会话去重锁争用不会丢失整段可用上下文。
5. Stop 流式识别长 JSONL 前导，避免超过 100 行 housekeeping 后误判 Codex 来源。
6. BGE-small-zh-v1.5 的实际输出为 512 维，修正 Rust 默认维数；没有改写现有关闭状态的模型配置。
7. Antigravity 增加显式项目上下文，避免全局 MCP 进程工作目录造成串库。
8. 补齐写入时更新 MEMORY.md、维护后重建该目录索引，保持文件型 agent 的读取入口可用。

## 真实模型与复跑

使用固定 Hugging Face revision 的量化 ONNX 文件：

- `Xenova/bge-small-zh-v1.5`，revision `75c43b069aac4d136ba6bc1122f995fedcfd2781`，512 维。
- `Xenova/ms-marco-TinyBERT-L-2-v2`，revision `b76bb5e1fefd66aa36cd108622d768e86c015ff1`。
- ONNX Runtime 1.30.0，来自官方 PyPI wheel；原生进程直接加载动态库，不运行 Python 推理。

模型保存在 `~/.cache/mnemosyne/models/` 对应目录，source.json 记录来源与 SHA-256。
验证仅使用合成记忆与临时 HOME，未将实际记忆送往模型服务，也未在真实库启用向量或重排。

```sh
python3 tests/native_onnx_smoke.py ~/.local/bin/mnemosyne \
  ~/.cache/mnemosyne/models/Xenova--bge-small-zh-v1.5/model.onnx - 512 \
  ~/.cache/mnemosyne/models/Xenova--ms-marco-TinyBERT-L-2-v2/model.onnx
python3 tests/native_http_models_smoke.py ~/.local/bin/mnemosyne
```

`-` 表示使用原生二进制旁的 ONNX Runtime。这些 Python 脚本只组织验收，内核仍是 Rust。

## 第一阶段的兼容边界

- `python3 -m mnemosyne` 与 `import mnemosyne.api` 仍是旧 Python 接口，尚无原生绑定；本次切换的是明确登记的 CLI/hooks/MCP 入口。
- 此阶段尚不支持跨 store 关系写入，后续阶段已补齐协调提交与恢复；交互式模糊合并与 consolidation 改为保守处理。
- Rust 不自动下载本地模型。旧 Python 的默认 BAAI ONNX 下载地址本次返回 404，不能把该旧路径当作已可用能力。
- 真实远端 LLM/embedding 提供商未调用，HTTP 协议测试不等于任意服务商实测。
- 500 条合成记录、release 独立进程对照：Python 冷建约 78ms、热查询 62–67ms；Rust 冷建约 501ms、热查询 59–62ms、单条编辑后约 58ms。冷建索引仍慢，不能笼统宣称 Rust 全面提速。
- 尚未执行 Linux/Windows 实机验收或托管 CI，也未开始后续历史/checkpoint/proposal 迭代。

## 第一阶段的数据与回滚

切换前在 store 锁下保存实际全局/项目 Markdown、配置和状态；没有直接拷贝活动 SQLite/WAL。
验收后再次比较：全局和项目可读记忆数量保持一致，实际 Markdown 新增/删除/改写均为 0。

备份保存在权限为 0700 的私有目录；其 manifest 记录原文件摘要和快照清单。
回滚工具默认只校验切换后的文件是否再次变化，显式应用时才恢复入口配置，
不会覆盖新产生的记忆。实际路径及操作脚本留在本机，不作为公开安装命令。

回滚后需刷新 Antigravity MCP。本阶段保留备份，未实际执行回滚，也尚未卸载旧 Python 包。

## Native-only completion, 2026-09-19

At this stage, the active runtime served Codex, Claude Code, Grok and Antigravity without the Python Mnemosyne package. Mnemosyne package registrations in system Python, the repository venv and uv tools were removed; the Python interpreters themselves were not retired by this work. Existing CLI entry points selected a versioned native binary. Python source and legacy tests were removed from the active tree; embedded templates/evaluation fixtures moved under `assets/`. Python stdlib smoke scripts are development checks only.

Validation: 44 Rust tests (33 library, 8 CLI, 3 native-only host protocols); clippy all features and formatting; SSE transport; local HTTP embedding/LLM mock; real ONNX embedding and cross-encoder. Native-only tests clear PATH and cover event wrappers, three transcript formats, replay, all eight MCP tools and two-project isolation. Cross-store relations additionally exercise five durable interruption stages and safe recovery.

Local acceptance: all 3 configured Codex hooks and 4 Claude/Grok hooks exited successfully; Grok inspect confirmed its four effective commands. Both Antigravity MCP configurations completed initialize, listed 8 tools and searched with explicit project_path from cwd `/`; live language_server children loaded the native binary and UI retained 8 tools enabled for both entries. These are configured-runner/protocol checks, not four newly created model conversations.

Global and project memory remained readable with unchanged record counts. Snapshot comparison preserved all memory bodies; a small number of global records had only normal access_count/last_accessed/strength changes during recall. No synthetic test memories were written to real stores. Existing model enablement and distillation policy were preserved.

Offline rollback uses a private archive and manifest verification. To intentionally restore the retired deployment, verify and restore the archived Python package first, restore the previous host commands second, then refresh Antigravity MCP. These recovery tools do not overwrite memory stores. Exact backup locations and recovery scripts remain private and offline; this report is not a transferable rollback script. No rollback was executed as part of acceptance.

Limits: no claim of Python import API compatibility; Hermes/OpenClaw are outside this four-host deployment. Pending cross-store transactions must recover before moving stores or changing the global root. Rare interrupted commits can retain small coordinator decision files. No upstream release had been published at this acceptance snapshot.
