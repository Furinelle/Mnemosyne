# Mnemosyne

[![CI](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml/badge.svg)](https://github.com/Furinelle/Mnemosyne/actions/workflows/ci.yml)

**Mnemosyne is a local-first memory kernel written in Rust.** Codex, Claude Code,
Grok Build and Antigravity share readable Markdown memories through a native CLI,
MCP tools and lifecycle events. Memory operations do not require Python, a
background daemon, an LLM account or a separate database server.

[中文文档](README.zh.md) · [Migration and compatibility](docs/rust-migration.md)

## Build and install

```sh
git clone https://github.com/Furinelle/Mnemosyne
cd Mnemosyne
cargo build --locked --release --features onnx
mkdir -p "$HOME/.local/bin"
cp target/release/mnemosyne "$HOME/.local/bin/mnemosyne"
export PATH="$HOME/.local/bin:$PATH"
mnemosyne --help
```

The `onnx` feature enables optional local model inference. Using those models
also requires a compatible ONNX Runtime shared library and existing model files
with `vocab.txt`. Put `libonnxruntime.dylib` (macOS) or `libonnxruntime.so` (Linux)
next to the executable, or set `ORT_DYLIB_PATH`. Models are not downloaded
automatically. Basic memory operations work without the library; omit
`--features onnx` when local inference is unnecessary.

Existing Python installations and host settings are not changed by building the
binary. Use its absolute path in host configuration to avoid resolving an old
executable on `PATH`.

## Quickstart

```sh
cd /path/to/your/project
mnemosyne init --agent codex
mnemosyne write --type codebase --importance 70 \
  --title "Auth service" --content "The auth token expires after 15 minutes."
mnemosyne search "auth token" --format json
mnemosyne prep "Investigate the auth callback"
```

Global memory lives in `~/.mnemosyne/` (`MNEMOSYNE_HOME` overrides it). Project
memory lives in `.mnemosyne/`. `core.md` holds the small shared core;
`working/*.md` and `archive/` hold individual memories. Markdown is authoritative;
SQLite search and vector caches can be rebuilt with `mnemosyne reindex` and
`mnemosyne embed-backfill`.

## Connect your host

| Host | Entry points | Setup suggestion |
|---|---|---|
| Codex | Session/prompt/Stop hooks, project instructions, CLI or MCP | `mnemosyne init --agent codex`; `mnemosyne install codex` |
| Claude Code | `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `Stop` hooks | `mnemosyne init --agent claude-code`; `mnemosyne install claude-code` |
| Grok Build | Claude-compatible hooks, project instructions, Grok JSONL distillation | `mnemosyne init --agent grok`; `mnemosyne install grok` |
| Antigravity | MCP with an explicit project directory | `mnemosyne init --agent antigravity`; `mnemosyne install antigravity` |

`init` creates project files and preserves existing instruction files. `install`
prints configuration suggestions; it does not edit host settings or claim that
the host has loaded them. Templates are in `assets/templates/`.

Start the native MCP server with `mnemosyne mcp serve`. Its eight tools are
`mnemosyne_search`, `mnemosyne_write`, `mnemosyne_read_core`, `mnemosyne_show`,
`mnemosyne_link`, `mnemosyne_graph`, `mnemosyne_maintain` and
`mnemosyne_prep_context`. Tools accept an absolute `project_path` when the host
launches the server outside the project, including Antigravity. Project requests
without a project context fail explicitly. Project configuration can restrict
MCP exposure using `mcp.expose_global` and `mcp.expose_project`.

For CLI integrations, send JSON on stdin:

```sh
printf '%s\n' '{"prompt":"Investigate the auth callback"}' |
  mnemosyne inject --event turn_start --session my-session --format json
```

| Neutral event | Input |
|---|---|
| `session_start` | `{}` |
| `turn_start` | `{"prompt":"..."}` |
| `file_touch` | `{"files":["src/auth.rs"]}` |
| `session_end` | `{"text":"..."}` or `{"transcript":{"path":"...","format":"auto"}}` |

`mnemosyne hook EVENT` supplies the Claude-compatible JSON envelope. Hooks never
emit a tool permission grant. `inject --fail-safe` reports errors on stderr and
returns empty stdout without blocking the host. Context budgets cover the
assembled output and are **estimated**, not exact tokenizer limits.

## Capabilities and boundaries

- SQLite FTS5 and CJK-aware lexical retrieval, optional vectors, RRF fusion,
  typed relation expansion and optional cross-encoder reranking.
- Source, recorded date, expiry and evidence metadata. Expired and superseded
  memories are excluded from default retrieval; a recorded date is not a
  verification date.
- Conservative writes: lexical similarity does not authorize dropping or
  superseding a changed fact. Explicit relations and consolidation remain
  available; consolidation previews candidates before changes.
- Strength decay per maintenance run, archiving and core candidates. Candidates
  do not automatically rewrite `core.md`.
- Opt-in transcript distillation for Claude, Codex, Grok, role JSONL and text.
  Message roles are preserved; reasoning and tool payloads are not evidence by
  default. `distill` previews unless `--commit` is given.
- Atomic Markdown writes, file locks and recoverable relation mutations,
  including coordinated cross-store links. Pending operations must recover
  before moving a store or changing the global coordinator location.

Endpoint addresses, credential environment-variable selectors and local model
paths are trusted only from global configuration. LLM extraction and model
inference are optional. The format contract is in [docs/interface.md](docs/interface.md),
findings exchange in [docs/handoff-format.md](docs/handoff-format.md), and host
mapping in [docs/adapters.md](docs/adapters.md).

The runtime is native-only. The previous Python package/import API and Hermes
Python MemoryProvider are retired. CLI aliases `codex-prep` / `codex-ingest`
and the `mnemosyne_codex_prep` MCP alias remain. See the
[migration guide](docs/rust-migration.md) before switching an existing deployment.

## Development and validation

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo run --locked -- eval run --min-recall 0.95
cargo run --locked -- eval run --pipeline full --min-recall 0.95
cargo run --locked -- eval run --longmemeval --pipeline full --min-recall 0.95
```

Templates and embedded evaluation fixtures are under `assets/templates/` and
`assets/eval/`. `tests/native_only.rs` runs the executable with an isolated home
and an empty `PATH`, checking event envelopes, transcript replay and MCP project
isolation. It does not replace acceptance inside a real host.

Python files named `tests/native_*_smoke.py` are optional **development test
harnesses**, using the standard library to drive the native executable. They
are not an installed kernel or a runtime dependency. Model smoke tests require
separately supplied local assets; fake-provider tests do not establish real
model quality. The embedded LongMemEval sample is not the full public benchmark.

[Validation record](docs/rust-validation.md) · [Local cutover record](docs/rust-local-cutover.md)
· [Changelog](CHANGELOG.md). These records describe their stated revisions and
scope, not a guarantee that every later build or host has been verified.

License: MIT.
