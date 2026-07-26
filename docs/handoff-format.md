# Mnemosyne Findings Exchange Format (v1)

Any agent can report new durable knowledge back to Mnemosyne by emitting a
*findings block*. `mnemosyne ingest` (alias: `codex-ingest`) parses the block
from stdin and writes each finding as a memory, with dedup and supersede
handling applied.

Two equivalent representations exist. `--format auto` (the default) picks the
right parser: input starting with `{` or `[` that parses as JSON is treated
as the JSON variant, everything else as Markdown.

## Markdown variant

Append the block at the END of your reply:

```markdown
**Findings:**
- type: pitfall
- importance: 72
- title: <=80 chars
- tags: tag1, tag2
- content: |
    Multiline content here.
    Indent every line with 4 spaces.
- evidence: optional short verbatim quote
```

Grammar rules:

- The header line is `**Findings:**` (Chinese alias: `**新发现:**`), on its
  own line.
- Each finding starts at its `- type:` line; repeat the whole field group for
  multiple findings.
- `content: |` opens an indented literal block. The indent of the first
  non-empty line sets the margin; the block ends at the first line indented
  less than that margin.
- Fields other than `type`, `importance`, `title`, `tags`, `content`,
  `evidence` are ignored.
- `type` must be one of the store's configured `memory.types`
  (default: `arch_decision`, `pitfall`, `codebase`, `preference`, `handoff`,
  `session_summary`). Unknown types are dropped with a warning on stderr.
- `importance` is clamped to 0–100. `title` is truncated to 80 chars.
  `evidence` is truncated to 200 chars.

## JSON variant

Either a bare array or an object with a `findings` key:

```json
{
  "findings": [
    {
      "type": "pitfall",
      "importance": 72,
      "title": "<=80 chars",
      "tags": ["tag1", "tag2"],
      "content": "Multiline content here.",
      "evidence": "optional short verbatim quote"
    }
  ]
}
```

Validation matches the Markdown variant: unknown `type` values are dropped
with a stderr warning, `importance` defaults to 50 and is clamped, empty
`title`/`content` drop the finding.

## Versioning

This document describes **v1**. The format is append-only: new optional
fields may be added in later versions, existing fields and the header
markers will not change meaning. Parsers must ignore unknown fields.
