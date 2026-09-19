# Native Rust migration

Mnemosyne's runtime is Rust-only. The executable performs memory operations
without spawning Python. The supported local host integrations are Codex,
Claude Code, Grok Build and Antigravity. The previous Python kernel, its import
API and the Hermes Python MemoryProvider are retired; they are not alternative
runtime backends in this source tree.

Building or installing a binary does not change an existing host's configured
command. The [local cutover record](rust-local-cutover.md) and
[validation record](rust-validation.md) describe their stated snapshots; they
must not be read as blanket acceptance of later changes.

## Build and runtime assets

```sh
cargo build --locked --release --features onnx
./target/release/mnemosyne --help
```

The `onnx` feature is optional. Without it, local ONNX inference returns an
explicit error; basic CLI/MCP memory operations and configured HTTP model
providers do not require ONNX. Local inference additionally needs compatible
ONNX Runtime, a model and its `vocab.txt`. The binary can discover
`libonnxruntime.dylib` on macOS or `libonnxruntime.so` on Linux beside itself;
`ORT_DYLIB_PATH` selects an explicit runtime library. There is no Python
fallback or automatic model download.

The Rust source is under `src/`. Configuration and host templates are under
`assets/templates/`; embedded retrieval fixtures are under `assets/eval/`.
These resources are compiled into the executable where needed. Cargo metadata
owns the native version. macOS and Linux are the primary build targets.

## Host entry points

| Host | Native setup |
|---|---|
| Codex | `mnemosyne init --agent codex`; `mnemosyne install codex` |
| Claude Code | `mnemosyne init --agent claude-code`; `mnemosyne install claude-code` |
| Grok Build | `mnemosyne init --agent grok`; `mnemosyne install grok` |
| Antigravity | `mnemosyne init --agent antigravity`; `mnemosyne install antigravity` |

`init` initializes project files without overwriting existing instructions.
`install` prints configuration suggestions; it does not modify host settings.
Use an absolute native binary path when applying the suggestions. Restart or
reload the affected host and verify its actual memory path before treating a
configuration change as accepted.

`mnemosyne hook SessionStart`, `UserPromptSubmit`, `PreToolUse` and `Stop`
provide Claude-compatible hook envelopes. The neutral API is
`mnemosyne inject --event session_start|turn_start|file_touch|session_end`.
Stop detects Claude, Codex and Grok transcript formats and preserves the source;
not every host exposes identical lifecycle hooks. The Codex suggestion omits
`PreToolUse`; its session, prompt and Stop hooks remain available. Codex/Grok
can also use the instruction-driven `prep` / `ingest` workflow.

MCP uses `mnemosyne mcp serve` over stdio; `--sse` is an optional transport.
All eight tools accept `project_path`. This is important for Antigravity and
other clients that start the server without a project working directory.
The path must be an absolute repository or initialized-project directory.
Without project context, project-scoped requests fail instead of implicitly
creating a store in the server's working directory. MCP exposure settings still
apply to the selected project.

## Storage and compatibility

Markdown remains authoritative. Rust uses separate disposable
`rust-index.sqlite` and `vectors-rust.sqlite` caches; an old store does not need
a bulk Markdown rewrite. Supported quoted list values and unknown flat metadata
are preserved. This is a supported frontmatter subset, not an arbitrary nested
YAML parser.

Before changing a live writer, stop participating writers and preserve the
affected store and a working rollback. Verify a restore into a separate directory.
Do not run an old writer against a pending Rust recovery operation. Direct file
editing remains supported, but unknown changes during recovery cause an explicit
stop rather than silently overwriting the file.

The CLI commands `codex-prep` and `codex-ingest`, and the MCP tool
`mnemosyne_codex_prep`, remain aliases. Human-readable output is not guaranteed
to be byte-identical; machine consumers should use JSON or MCP. Previous Python
imports, Python hook module commands and the Hermes provider are no longer
supported. Existing scripts that use them must switch to the native CLI/MCP;
leaving an old pip-installed package on a machine does not migrate that caller.

For the persistent contracts, see [interface.md](interface.md),
[handoff-format.md](handoff-format.md) and [adapters.md](adapters.md).

## Deliberate behavior changes

1. Similarity alone never supersedes or discards a changed fact. Exact duplicate
   checks include type, body, tags, expiry and evidence. Explicit
   `link --rel supersedes` remains available and is idempotent.
2. Consolidation previews approximate candidates. Commit only supersedes exact
   equivalent records and retains old Markdown and links.
3. Relation mutations use durable recovery journals; cross-store links use a
   coordinated commit decision. Pending operations must recover before moving
   stores or changing the global coordinator location. Unknown external changes
   stop recovery rather than being overwritten. This protocol is not the future
   semantic revision-history system or proposal ledger.
4. Context budgets count the complete assembled output and are explicitly
   estimated. Mandatory core exceeding the budget raises `BUDGET_TOO_SMALL`;
   fail-safe hooks emit empty context and a diagnostic.
5. Vector caches check input hash, model fingerprint and dimension. Backfill
   checks the source again after computation. These checks cannot reveal an
   unannounced remote provider weight change behind an unchanged model alias.
6. Endpoint addresses, credential environment-variable selectors, local model
   paths and SSE binding configuration retain their global-only trust boundary.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo run --locked -- eval run --min-recall 0.95
cargo run --locked -- eval run --pipeline full --min-recall 0.95
cargo run --locked -- eval run --longmemeval --pipeline full --min-recall 0.95
```

`tests/native_only.rs` drives the actual native binary with cleared environment,
isolated stores and an empty `PATH`. It covers hook envelopes, transcript source
and replay handling, and Antigravity-style MCP project switching and exposure.
These are protocol fixtures, not claims of a live desktop host test.

Optional development smoke scripts use Python's standard library as an external
test driver. They do not import a Mnemosyne Python kernel or become runtime
requirements:

```sh
python3 tests/native_transport_smoke.py target/release/mnemosyne
python3 tests/native_http_models_smoke.py target/release/mnemosyne
# Requires existing local model, vocabulary and runtime library:
python3 tests/native_onnx_smoke.py target/release/mnemosyne \
  /path/to/model.onnx /path/to/libonnxruntime.dylib 512
```

Report local model tests separately from fake HTTP-provider tests and tokenizer
unit tests. The fixed retrieval gate is a regression check, not a general quality
promise. The embedded LongMemEval sample is not the complete public benchmark.
Use a pinned external corpus to claim results on that benchmark, and distinguish
retrieval scores from answer correctness.

The subsequent evolution roadmap remains separate work: provenance, semantic
revisions, checkpoints, system-time history, code applicability and approved
maintenance proposals. Native migration does not imply these features exist.
