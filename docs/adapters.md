# Adapter Contract

An *adapter* connects one agent host to the Mnemosyne kernel. Adapters are
thin: they translate the host's native lifecycle (hooks, plugin callbacks,
wrapper scripts) into four neutral injection events, and optionally install
themselves via `mnemosyne install <agent>`.

Reference implementations: `mnemosyne/integrations/claude_code/` (hook
protocol shell) and `mnemosyne/integrations/hermes/` (in-process plugin via
`integrations/_bridge.CLIBridge`).

## The four events

Call them through `mnemosyne.events.handle_event(event, payload, session=,
channel=)` (Python) or `mnemosyne inject --event <name> [--session ID]
[--channel cli|mcp|none] [--format text|json] [--fail-safe]` with the payload
as JSON on stdin (CLI).

| Event | Fire when | Payload | Result |
|---|---|---|---|
| `session_start` | a session opens | `{}` | core memory block (`## Mnemosyne Memory` + global/project core) |
| `turn_start` | the user submits a prompt | `{"prompt": str}` | ≤3 relevant memories, one line each |
| `file_touch` | the agent is about to modify files | `{"files": [str, ...]}` | ≤2 memories per file basename |
| `session_end` | the session closes | `{"text": str}` or `{"transcript": {"path": str, "format": "auto\|claude-jsonl\|role-jsonl\|text"}, "source": str}` | auto-distill summary (requires `[distill].enabled = true`) |

`InjectionResult` / JSON output: `{"context": str, "memory_ids": [str],
"approx_tokens": int}`. Empty `context` means "inject nothing".

## Contract rules

1. **Fail-safe**: an adapter must never block or crash its host. Exit 0 and
   inject nothing on any error (`inject --fail-safe`, or `hook_safe()` in
   Python). Never print diagnostics to the channel the host treats as
   injected context.
2. **Session key**: pass the host's session identifier (any stable string)
   as `session` so cross-turn dedup works. Without it, dedup degrades
   gracefully — every call injects fresh, and the context budget pays for it.
3. **Channel**: pass `channel="mcp"` when the agent can only call MCP tools,
   `"cli"` when it has a shell, `"none"` to suppress the retrieval hint. The
   footer of injected lists adapts (`injection.show_command_template`
   overrides globally).
4. **Source naming**: tag writes with `<agent>[:<profile>]`, lowercase
   (e.g. `claude-code`, `hermes:coder`). `write`/`ingest`/`distill`
   normalize and warn on other shapes.
5. **Transcripts**: to use full distillation, feed `session_end` a
   transcript. If your host's format is not built in, preprocess to
   *role-jsonl* — one `{"role": "user"|"assistant", "text": "..."}` object
   per line.
6. **Host policy stays in the adapter**: auto-init, maintenance scheduling,
   re-entrancy guards, and anything host-specific belong in your adapter,
   not in the kernel (see `integrations/claude_code/session_start.py`).
7. **Host builtin memory**: if your host has its own memory subsystem,
   installing Mnemosyne does *not* automatically disable it — switch the
   host's provider explicitly (e.g. Hermes `memory.provider: mnemosyne`),
   or you will get double injection.

## Installers

Register a function in `mnemosyne/integrations/_registry.py::INSTALLERS`
(name → `fn(argparse.Namespace) -> int`) to make `mnemosyne install <agent>`
work. Installers should support `--dry-run` and be idempotent (or require
`--force` to overwrite).

## In-process plugins

If your adapter lives inside another process (like the Hermes provider), use
`mnemosyne.integrations._bridge.CLIBridge`: it finds a Python interpreter
that can import mnemosyne (override with `MNEMOSYNE_BRIDGE_PYTHON`), shells
out with a timeout, and returns empty results instead of raising.

## Conformance

Run the contract suite against your adapter's event mapping:

```bash
python3 -m pytest tests/test_adapter_contract.py -q
```

An adapter is conformant when: all four events return well-formed results,
findings round-trip through `ingest`, repeated distillation is idempotent
(dedup marks duplicates), and concurrent writes stay safe (the kernel locks;
just do not bypass the CLI/API to write files directly without locking).
