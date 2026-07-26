# Mnemosyne 记忆系统使用规则

本项目启用 Mnemosyne，长期记忆通过 Claude Code hooks 自动注入与持久化。

## 任务开始时

- **Core memory** 已由 `SessionStart` hook 自动注入，无需手动 `read`。
- **Prompt 相关历史 memory** 已由 `UserPromptSubmit` hook 自动注入。
- **Edit/Write 时**，与目标文件 basename 相关的 memory 已由 `PreToolUse` hook 自动注入。

如需主动查更多上下文：
    python3 -m mnemosyne search "<keywords>" --format json --limit 5

## 任务结束前 — 主动写入触发条件

遇到以下情况，**任务结束前必须**调用 `python3 -m mnemosyne write --force`：

### 1. 踩坑 / Bug 修复（`--type pitfall`，importance 70-85）
触发条件：debug 花了 >5 分钟，根因和修复方案值得记录。
内容必须含：现象 / 根因 / 修复 / 复现步骤 / 如何避免。

### 2. 架构决策（`--type arch_decision`，importance 75-90）
触发条件：选 A 不选 B 且理由非显然。
内容必须含：选项 / 选择 / 理由 / 被否选项的劣势。

### 3. 用户偏好（`--type preference`，importance 60-80，`--scope global`）
触发条件：用户纠正了你的默认行为（如"不要用 X，用 Y"）。
注意：只写明确的、可复用的偏好；一次性指示不写。

### 4. 代码库知识（`--type codebase`，importance 50-70）
触发条件：花了 >3 个工具调用才搞清楚某模块的职责或入口。
内容必须含：模块名 / 职责 / 关键文件 / 调用关系。

### 5. Codex 交接（`--type handoff`，importance 60-80）
触发条件：调用 Codex 后从 `**新发现:**` 段落得到信息。
注意：如果用了 `python3 -m mnemosyne codex-ingest` 会自动写，此时**不要**手动再写。

## 写入命令模板

    python3 -m mnemosyne write \
      --type <type> --importance <n> --source claude-code --force \
      --title "<≤80 字>" \
      --tags "<逗号分隔>" \
      --content "<结构化内容>"

## 不要写入的情况

- 普通对话或闲聊
- 一次性的任务结果（如"我帮你删了文件 X"）
- 已经在 core.md 里的常识
- 不确定能否复用的内容
- 跟其它已有 memory 高度重复（先 `search` 一下确认）

## 关于自动蒸馏（`[distill].enabled = true`）

若 `.mnemosyne/config.toml` 中 `[distill].enabled = true`，Stop hook 会在会话结束时
自动从对话中提炼并保存记忆（启发式或可选的 LLM 引擎，见 `engine` 配置项）。
开启此功能后，上面的手动 `write` 仅用于补充自动蒸馏漏掉的内容，不必每次任务都手动调用。
