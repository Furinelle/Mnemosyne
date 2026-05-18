# Mnemosyne

Portable, general-purpose memory for LLM agents. It is designed for agents that
can call a shell and Python, including Claude Code, Codex, and similar tools.

Mnemosyne uses only the Python standard library and requires Python 3.10 or
newer.

## Quick Start

Initialize project memory in the current directory:

```bash
python -m mnemosyne init
```

Read core memories for prompt injection:

```bash
python -m mnemosyne read --scope all
```

Write a memory:

```bash
python -m mnemosyne write --type pitfall --importance 70 --title "UTF-8 on Windows" --content "Use UTF-8 for shell text operations."
```

Search memories:

```bash
python -m mnemosyne search "UTF-8 Windows" --scope all
```

Run maintenance:

```bash
python -m mnemosyne maintain --scope all --dry-run
```

## Storage

Global memory is stored in `MNEMOSYNE_HOME`, defaulting to `~/.mnemosyne/`.

Project memory is stored in `.mnemosyne/`. Commands auto-detect a project store
by walking upward from the current directory until `.mnemosyne/` is found. If no
store is found, the walk stops at the git root.

## Configuration

Configuration priority is:

1. Environment variables
2. `.mnemosyne/config.toml`
3. Defaults

Optional TOML config loading uses `tomllib` on Python 3.11 and newer. On Python
3.10, config files are skipped silently.

Default config:

```toml
[thresholds]
decay_per_run = 1
bonus_access = 5
bonus_write = 10
bonus_recall = 20
core_strength = 80
core_access_count = 3
archive_strength = 30
deprecated_strength = 5

[memory]
types = ['arch_decision', 'pitfall', 'codebase', 'preference', 'handoff']
```

## CLI

```bash
python -m mnemosyne --help
```

Subcommands:

- `init`
- `read`
- `write`
- `search`
- `maintain`
- `show`
- `link`

