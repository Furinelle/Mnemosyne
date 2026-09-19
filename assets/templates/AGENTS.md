# Agent Coordination via Mnemosyne

For work that depends on project history, load missing context from the current
project directory. Skip retrieval when relevant memory is already included or
the question is self-contained. Verify time-sensitive facts against current state.

```sh
mnemosyne read --scope all
mnemosyne search "<task keywords>" --format json --limit 3
mnemosyne show <memory-id>
```

Do not assume hooks have injected memory merely because this instruction file
exists. Confirm the context actually delivered by the host.

## Writing and reporting findings

Follow the host's memory-write policy and the user's authorization. When writing
is authorized, save only verified, reusable findings after checking for duplicates.
Never persist secrets, speculation, one-off results or restatements of the task.

When a handoff consumer is configured to ingest findings, report:

```text
**新发现:**
- type: pitfall|arch_decision|codebase|handoff
- importance: 50-90
- title: <=80 chars
- tags: tag1, tag2
- content: |
    Verified finding, indented four spaces.
```

A consumer must run `mnemosyne ingest --commit` to save the block. Reporting it
is not proof of persistence. Check the command result before claiming a save.

Automatic session distillation requires an installed hook, an enabled
`[distill].enabled` setting and permission under the host's memory-write policy.
Do not enable it or infer successful ingestion from this template alone.
