# Mnemosyne project context

Use current source and tests as the authority. Do not assume a prompt prefix has already loaded memory: if relevant history is missing, run `python3 -m mnemosyne read --scope all` and a focused `python3 -m mnemosyne search "<keywords>" --format json --limit 5`.

Follow the active host's memory-write policy. Save only verified, reusable findings after checking for duplicates; exclude secrets, speculation, one-off task results, and restatements of the request. A findings block is saved only if a consumer actually runs `codex-ingest --commit`.
