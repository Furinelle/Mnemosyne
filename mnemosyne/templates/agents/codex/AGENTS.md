# Mnemosyne for Codex

For work that depends on project history, load missing context from the current project directory:

```bash
python3 -m mnemosyne read --scope all
python3 -m mnemosyne search "<task keywords>" --format json --limit 5
```

Skip this when relevant memory is already included or the question is self-contained. Verify time-sensitive facts against current state.

Follow the host's memory-write policy. Report only verified, reusable findings after checking for duplicates; omit secrets, speculation and one-off task results. When the handoff consumer is configured to ingest findings, use:

```text
**新发现:**
- type: pitfall|arch_decision|codebase|handoff
- importance: 50-90
- title: <=80 chars
- tags: tag1, tag2
- content: |
    Verified finding, indented four spaces.
```

A consumer must run `python3 -m mnemosyne codex-ingest --source codex --commit` to save these blocks. Do not claim automatic memory injection or successful ingestion without evidence.
