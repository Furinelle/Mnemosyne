# Long-Term Memory via Mnemosyne

This project uses [Mnemosyne](https://github.com/Furinelle/Mnemosyne) as a
shared, local-first memory store. Memories are plain Markdown files under
`.mnemosyne/` (project) and `~/.mnemosyne/` (global) — you can read them
directly, and `core.md` is the always-relevant summary worth loading first.

## Reading memory

Pick whichever channel you have:

- **CLI** (any agent with a shell):

      python3 -m mnemosyne search "<keywords>" --format json --limit 3
      python3 -m mnemosyne show <memory-id>

- **MCP**: connect to the server (`python3 -m mnemosyne mcp serve`) and call
  the `mnemosyne_search` / `mnemosyne_show` / `mnemosyne_read_core` tools.

- **Files**: grep the Markdown files under `.mnemosyne/working/` directly.

## Writing memory

Record durable knowledge (pitfalls, architecture decisions, user preferences,
codebase facts) — not one-off task details:

    python3 -m mnemosyne write --type <pitfall|arch_decision|codebase|preference> \
      --importance <50-90> --title "<short title>" --tags "tag1,tag2" \
      --content "<structured content>"

Or via MCP: the `mnemosyne_write` tool.

## Reporting findings (handoff)

When another agent prepared your task with `mnemosyne prep`, report new
discoveries by appending a findings block at the END of your reply — either
Markdown or JSON (see docs/handoff-format.md in the Mnemosyne repository):

**Findings:**
- type: pitfall
- importance: 70
- title: <=80 chars
- tags: tag1, tag2
- content: |
    Multiline content, 4-space indent.

Skip the block if there is nothing durable to record.
