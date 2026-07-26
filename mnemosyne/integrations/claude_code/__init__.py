"""Claude Code adapter: maps Claude Code hook events onto mnemosyne.events.

Reference implementation of the adapter contract (docs/adapters.md). The
modules here are thin protocol shells: parse the Claude Code hook JSON from
stdin, call handle_event, and wrap the result in hookSpecificOutput. Host
policy (auto project init, maintenance scheduling, stop re-entrancy) also
lives here because it is Claude-Code-session specific.
"""
