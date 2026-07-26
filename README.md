# Mnemosyne

[![CI](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml/badge.svg)](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml)

> Greek goddess of memory, mother of the nine Muses.

**Mnemosyne is a local-first, agent-agnostic memory kernel.** Long-term
memories are plain Markdown files on your disk, indexed by SQLite FTS5 hybrid
retrieval, shared by every agent you use — Claude Code, Codex CLI, Cursor,
Hermes, or anything that can speak MCP, run a shell command, or read a file.

[中文文档 → README.zh.md](README.zh.md)

## Why Mnemosyne

Most agent memory systems make you choose between heavy infrastructure
(Postgres + vector DB + Docker), an LLM key on every write, or a single
vendor's ecosystem. Mnemosyne refuses all three:

- **Files are the source of truth.** Every memory is Markdown + YAML
  frontmatter. Read it, grep it, `git diff` it, edit it by hand. Your
  memories stay readable forever, with or without Mnemosyne.
- **Zero heavy dependencies.** Python 3.11+ and `portalocker`. Retrieval is
  SQLite FTS5 (CJK-aware hybrid search, optional vector lane and reranker);
  it degrades gracefully to a pure-stdlib BM25 when FTS5 is unavailable.
- **No LLM in the loop by default.** Auto-distillation is heuristic-first;
  an LLM engine is opt-in, never required.
- **Agent-neutral core, adapters at the edge.** One kernel, one store, many
  doors: MCP tools, a CLI, lifecycle injection events, and direct file access.

## Quickstart

```bash
git clone https://github.com/Furinelle/Mnemosyne && cd Mnemosyne
pip install -e .
cd /path/to/your/project
python3 -m mnemosyne init          # creates .mnemosyne/ + a generic AGENTS.md
python3 -m mnemosyne write --type pitfall --importance 70 \
  --title "Example" --content "The auth token expires after 15 minutes."
python3 -m mnemosyne search "auth token"
```

Global preferences live in `~/.mnemosyne/`, project knowledge in the
repository's `.mnemosyne/`.

## Integrate any agent

Three access paths, pick what your agent has. All of them share the same
store and the same retrieval kernel.

### 1. MCP (works with Claude Code, Codex CLI, Cursor, Cline, Windsurf, Gemini CLI, …)

```bash
python3 -m mnemosyne mcp serve      # stdio, stdlib-only, no extra installs
```

Tools: `mnemosyne_search`, `mnemosyne_write`, `mnemosyne_read_core`,
`mnemosyne_show`, `mnemosyne_link`, `mnemosyne_graph`, `mnemosyne_maintain`,
`mnemosyne_prep_context`. Client config snippets for Cursor / Cline /
Continue / Windsurf are in `mnemosyne/templates/mcp_clients/`.

A pure-MCP client gets a full workflow with two calls: `mnemosyne_read_core`
at session start, `mnemosyne_search` whenever context is needed.

### 2. CLI + injection events (any agent with a shell or hooks)

Every capability is a subcommand (`write`, `search`, `show`, `link`,
`graph`, `distill`, `prep`, `ingest`, …) with `--format json` output where
it matters. On top of that, four agent-neutral lifecycle events power
automatic injection:

| Event | When | Payload (stdin JSON) |
|---|---|---|
| `session_start` | session opens | `{}` → core memory block |
| `turn_start` | user submits a prompt | `{"prompt": "..."}` → relevant memories |
| `file_touch` | agent is about to edit files | `{"files": ["a.py"]}` → file-relevant memories |
| `session_end` | session closes | `{"text": ...}` or `{"transcript": {"path": ...}}` → auto-distill |

```bash
echo '{"prompt": "why does portalocker deadlock"}' | \
  python3 -m mnemosyne inject --event turn_start --session my-session --format json
```

`--fail-safe` makes it exit 0 with empty output on any error, so a hook can
never block the host. See [docs/adapters.md](docs/adapters.md) for the full
adapter contract.

### 3. Files (any agent that can read)

`.mnemosyne/core.md` is the always-relevant summary; one Markdown file per
memory lives under `.mnemosyne/working/`. The frontmatter schema and the
`search --format json` output schema are specified in
[docs/interface.md](docs/interface.md). Findings exchange (agent → memory)
is specified in [docs/handoff-format.md](docs/handoff-format.md), with
Markdown and JSON variants.

## Official adapters

| Agent | Integration | Install |
|---|---|---|
| Claude Code | Lifecycle hooks (SessionStart / UserPromptSubmit / PreToolUse / Stop) with deterministic injection + auto-distill | `python3 -m mnemosyne install claude-code` (prints the merge steps) |
| Codex CLI | `AGENTS.md` protocol + `prep` / `ingest` handoff blocks | `python3 -m mnemosyne init --agent codex` |
| Hermes | Native MemoryProvider plugin (inject, recall, tool, distill) | `python3 -m mnemosyne install hermes` |

Adapters are peers on top of the same neutral events — writing one for
another agent is a thin mapping layer; see [docs/adapters.md](docs/adapters.md).

## Feature highlights

- **Hybrid retrieval**: SQLite FTS5 with CJK bigram handling, optional
  embedding lane fused via RRF, optional cross-encoder rerank, typed-link
  graph expansion.
- **Memory lifecycle**: strength decay, archiving, recall bonus, expiry,
  core-promotion candidates, near-duplicate consolidation.
- **Typed relations & graph**: `caused_by` / `refines` / `supersedes` /
  `contradicts` / `related`, with Mermaid / ASCII / JSON rendering.
- **Temporal validity**: superseded memories are marked invalid but kept;
  default retrieval filters them, `--include-superseded` looks back.
- **Auto memory formation**: opt-in `distill` extracts durable memories from
  transcripts (`claude-jsonl`, neutral `role-jsonl`, plain text) with dedup
  and supersede handling before every write.
- **Reproducible evaluation**: `eval run` reports recall/MRR on a fixed
  corpus; `eval convert longmemeval` + `eval run --longmemeval` benchmark
  against LongMemEval; CI enforces a recall@5 regression gate.
- **Concurrency-safe by design**: atomic writes, `portalocker` lock
  ordering, SQLite WAL — multiple agents can share one store.

## Stability promises

- Memory files stay readable: the storage format is stable and versioned
  changes are append-only.
- The `mnemosyne.api` module is the public Python API.
- Legacy surfaces (`codex-prep` / `codex-ingest` / `install-hermes`
  commands, `mnemosyne.codex` / `mnemosyne.hooks.*` imports, the
  `mnemosyne_codex_prep` MCP tool) remain as aliases.

## Changelog

Current version: 0.7.0 — the "universal memory kernel" release: stable
`mnemosyne.api`, neutral injection events (`mnemosyne inject`), JSON
findings variant, transcript parser registry, per-agent adapters and
templates, English-first docs. Storage format unchanged; every legacy
command, import path, and MCP tool name keeps working via aliases. See
[CHANGELOG.md](CHANGELOG.md).

## Development

```bash
python3 -m pytest tests/ -q
python3 -m mnemosyne doctor
```

License: MIT. Contributions welcome — run the test suite and keep the
stdlib-only core constraint.
