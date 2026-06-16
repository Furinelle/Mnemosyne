# Agent Coordination via Mnemosyne

When you receive a task, the prompt prefix already includes:
1. Global and project core memory
2. Relevant prior memories matched against your task keywords

If you need more context during the task:

    python -m mnemosyne search "<keywords>" --format json --limit 3

## When you finish - report new findings

If you discovered something worth persisting, append a block in this exact format
at the END of your reply (multiple blocks allowed; skip if there is nothing):

**新发现:**
- type: pitfall|arch_decision|codebase|handoff
- importance: 50-90
- title: <=80 chars
- tags: tag1, tag2
- content: |
    multi-line content here
    keep 4-space indent under "content: |"

Claude Code will automatically ingest these via:

    python -m mnemosyne codex-ingest --source codex --commit

## Do NOT report

- One-off task results
- Speculative or unverified content
- Anything already in core memory
- Restatements of the task itself

## Auto-distill

For Codex, emitting the `**新发现:**` block above and running
`python -m mnemosyne codex-ingest --commit` is the Codex-side auto-distill path.
Hermes does this natively after `install-hermes`: its MemoryProvider's
`on_session_end()` hook auto-distills the finished conversation when
`[distill].enabled = true` in the shared Mnemosyne config — no manual block needed.
