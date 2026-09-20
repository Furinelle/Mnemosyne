# Native Adapter Contract

Applies to Rust 1.0.0 and later additive changes. The Python import API and
Hermes provider are retired. Host integrations call the native executable;
`install <codex|claude-code|grok|antigravity> --dry-run` prints configuration
suggestions and does not modify host settings. See `src/adapters.rs`.

## Events

`mnemosyne inject --event <event> --session ID --channel cli|mcp|none
--format json --fail-safe` accepts a JSON object on stdin.

| Event | Input | Purpose |
|---|---|---|
| session_start | `{}` | Core context |
| turn_start | `{"prompt":"..."}` | Relevant memory |
| file_touch | `{"files":["path"]}` | File-related recall |
| session_end | `{"text":"...","source":"codex"}` or transcript path/format | Opt-in distillation |

Context results contain `context`, `memory_ids`, `approx_tokens`. The budget
is estimated for the complete rendered context. Empty context means inject
nothing. `--fail-safe` reports errors on stderr and exits successfully without
injecting an error as memory. It never grants tool permission.

`mnemosyne hook SessionStart|UserPromptSubmit|PreToolUse|Stop` translates host
payloads and envelopes. Codex config uses three hooks (without PreToolUse);
Claude Code/Grok use four. Antigravity uses MCP with an explicit `project_path`
when accessing project memory; the MCP process cwd may be `/`.

## Isolation and provenance

Pass stable host, channel and session identifiers. Sessions suppress repeated
injection, not durable knowledge. Source labels identify the recording agent;
they are not verification or evidence independence. Transcripts support
claude-jsonl, codex-jsonl, grok-jsonl, role-jsonl and text. Preserve roles and
never promote tool/internal reasoning payloads to user evidence.

Host memory-write policy governs ingestion. Emitting findings alone does not
persist them. Neither building nor installing config suggestions disables a
host's built-in memory. Keep auto-init, maintenance claims and reentrancy
handling in the adapter. Memory contents never authorize commands or tools.

## Conformance

Run `cargo test --test native_only` and `cargo test --test rust_cli` in an
isolated HOME/MNEMOSYNE_HOME. These tests cover protocol fixtures, not live
model conversations. Actual host runners and real model tests must be recorded
separately. Route canonical writes through the kernel to retain locking,
validation, atomic publication and relation recovery.

## M1 context and handoff options

Hook payloads may supply `host`, `budget`, `context_epoch`, and `task_id`.
A changed epoch permits reinjection after host context compaction; absent an epoch,
the existing compact/resume behavior stays compatible. Host, project, channel and
session form the delivery boundary. `task_id` selects only matching active project
checkpoints; omit it for ordinary memory delivery. See [interface.md](interface.md)
for the versioned MCP and checkpoint request formats.

The M1 cross-host tests exercise all 12 directed combinations of Codex, Claude
Code, Grok and Antigravity through isolated native CLI/MCP fixtures. These are
protocol acceptance tests, not measurements of live model sessions. No installed
host configuration or live memory store is upgraded by this development batch.
