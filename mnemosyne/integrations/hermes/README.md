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
