# Mnemosyne memory provider for Hermes

Bridges Hermes to the shared Mnemosyne store (the same one Claude Code and
Codex use), so long-term memory is shared across all three agents.

## Install

    python3 -m mnemosyne install-hermes

This copies the provider into `$HERMES_HOME/plugins/mnemosyne/` and sets
`memory.provider: mnemosyne` (plus a `plugins.mnemosyne` block) in
`config.yaml`. Restart Hermes to activate.

## Config (config.yaml)

    memory:
      provider: mnemosyne
    plugins:
      mnemosyne:
        python: /opt/homebrew/bin/python3   # a python that can `import mnemosyne`
        recall_limit: 5
        timeout: 5
        mirror_builtin_writes: false

## Behavior

- System prompt: injects global + project core memory (`mnemosyne read`).
- Each turn: recalls top matches (`mnemosyne search`).
- Tool `mnemosyne`: search / write / show / link / graph against the shared store.
- Writes are tagged `--source hermes[:<profile>]`.
- Session end: if `[distill].enabled = true` in the shared Mnemosyne config,
  `on_session_end()` bridges to `mnemosyne distill --stdin --commit --source hermes[:<profile>]`
  to auto-extract durable memories from the finished conversation (same pipeline
  Claude Code's Stop hook uses). Off by default; no-op when disabled.
