# Mnemosyne 记忆使用规则

Mnemosyne 提供项目与全局共享记忆。本文件不代表 hooks 已安装、记忆已注入或自动写入已获授权。

## 读取上下文

任务依赖项目历史且当前上下文缺少相关信息时，从项目目录按需读取：

```sh
mnemosyne read --scope all
mnemosyne search "<任务关键词>" --format json --limit 5
mnemosyne show <memory-id>
```

自包含问题或已有足够记忆时跳过检索。记录日期不等于最近核查日期，易变化事实应对照当前状态。
若 hooks 已安装且实际返回上下文，可以复用其内容，不必重复读取。

## 写入与交接

遵守宿主的记忆写入政策和用户授权。只有写入已获允许时，才保存经验证、可复用且不重复的知识：

- `pitfall`：非显然的故障根因、修复与复现条件。
- `arch_decision`：架构选择、理由及适用范围。
- `preference`：明确的长期偏好，通常使用 `--scope global`。
- `codebase`：稳定的项目入口、模块职责和依赖关系。
- `handoff`：经核实且适合后续任务复用的交接信息。

```sh
mnemosyne write --type <type> --importance <50-90> --source claude-code \
  --title "<简短标题>" --tags "tag1,tag2" --content "<核实后的内容>"
```

先检索去重；不保存秘密、推测、一次性结果或请求复述。检查命令结果后再声称保存成功。
若已由消费者通过 `mnemosyne ingest --commit` 成功写入 findings，不要重复写入；
仅输出 findings 块并不代表它已经持久化。

## 自动蒸馏

自动蒸馏需要实际安装的 Stop hook、`[distill].enabled = true`，并符合宿主写入政策。
不要因为此模板存在而自行启用，也不要假定每次会话结束均已成功写入。
