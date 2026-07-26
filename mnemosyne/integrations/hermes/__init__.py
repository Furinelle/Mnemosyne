"""Hermes MemoryProvider that bridges to the shared Mnemosyne store via CLI.

Installed into ``$HERMES_HOME/plugins/mnemosyne/`` by
``python3 -m mnemosyne install-hermes``. Activated via ``memory.provider:
mnemosyne`` in Hermes ``config.yaml``.

Importable standalone (for tests) — the Hermes-only base class and helpers
fall back to local shims when Hermes is not on the path.
"""
from __future__ import annotations

import json
import logging
from typing import Any, Dict, List, Optional

from mnemosyne.integrations._bridge import CLIBridge

logger = logging.getLogger(__name__)

try:  # Hermes runtime
    from agent.memory_provider import MemoryProvider  # type: ignore
except Exception:  # standalone / tests
    class MemoryProvider:  # minimal shim
        pass

try:  # Hermes runtime
    from tools.registry import tool_error  # type: ignore
except Exception:  # standalone / tests
    def tool_error(msg: str) -> str:
        return json.dumps({"error": msg})


def _message_text(content: Any) -> str:
    if isinstance(content, str):
        return content.strip()
    if isinstance(content, list):
        parts = []
        for block in content:
            if isinstance(block, dict) and block.get("type") == "text":
                parts.append(str(block.get("text", "")))
        return "\n".join(p for p in parts if p).strip()
    return ""


def _messages_to_text(messages: Optional[List[Dict[str, Any]]]) -> str:
    """Render OpenAI-style messages as ``[role] text`` lines for distill_text."""
    lines: List[str] = []
    for message in messages or []:
        role = message.get("role")
        if role not in ("user", "assistant"):
            continue
        text = _message_text(message.get("content"))
        if text:
            lines.append(f"[{role}] {text}")
    return "\n\n".join(lines)


MNEMOSYNE_TOOL: Dict[str, Any] = {
    "name": "mnemosyne",
    "description": (
        "Shared long-term memory for all agents via Mnemosyne. "
        "action='search' to recall past memories; action='write' to persist a "
        "durable fact the user would expect remembered across sessions and agents "
        "(preferences, decisions, pitfalls, codebase knowledge). "
        "Also: show / link / graph."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["search", "write", "show", "link", "graph"]},
            "query": {"type": "string", "description": "search query (action=search)"},
            "limit": {"type": "integer", "description": "max results (action=search)"},
            "type": {"type": "string", "description": "preference|arch_decision|pitfall|codebase|handoff (action=write)"},
            "importance": {"type": "integer", "description": "50-90 (action=write)"},
            "scope": {"type": "string", "enum": ["global", "project"], "description": "write scope (default global)"},
            "title": {"type": "string"},
            "content": {"type": "string"},
            "tags": {"type": "string", "description": "comma-separated"},
            "id": {"type": "string", "description": "memory id (action=show|graph|link)"},
            "id2": {"type": "string", "description": "second id (action=link)"},
            "rel": {"type": "string", "description": "caused_by|refines|supersedes|contradicts|related (action=link)"},
        },
        "required": ["action"],
    },
}


class MnemosyneMemoryProvider(MemoryProvider):
    """Bridges Hermes memory lifecycle to the shared Mnemosyne CLI."""

    def __init__(self, config: Optional[Dict[str, Any]] = None):
        self._config = config or {}
        self._bridge_instance: Optional[CLIBridge] = None
        self._session_id = ""
        self._source = "hermes"
        self._recall_limit = 5
        self._timeout = 5.0
        self._mirror = False

    @property
    def name(self) -> str:
        return "mnemosyne"

    # -- bridge ---------------------------------------------------------------

    def _bridge(self) -> CLIBridge:
        if self._bridge_instance is None:
            explicit = self._config.get("python")
            if explicit:
                # Respect an explicitly configured python; do not silently fall
                # back to a different interpreter (which may see a different store).
                self._bridge_instance = CLIBridge(
                    python_candidates=[explicit], timeout=self._timeout, allow_env_override=False
                )
            else:
                self._bridge_instance = CLIBridge(timeout=self._timeout)
        return self._bridge_instance

    def _resolve_python(self) -> Optional[str]:
        return self._bridge().resolve()

    def _run(self, args: List[str], *, json_out: bool = False, input_text: Optional[str] = None):
        if json_out:
            payload = self._bridge().run_json(args, stdin=input_text)
            return payload if payload is not None else []
        code, out, err = self._bridge().run(args, stdin=input_text)
        if code != 0:
            logger.debug("mnemosyne %s exit %s: %s", args[:1], code, err[:200])
            return ""
        return out

    # -- lifecycle ------------------------------------------------------------

    def is_available(self) -> bool:
        return self._resolve_python() is not None

    def initialize(self, session_id: str, **kwargs) -> None:
        self._session_id = session_id or ""
        self._recall_limit = int(self._config.get("recall_limit", 5))
        self._timeout = float(self._config.get("timeout", 5))
        self._mirror = bool(self._config.get("mirror_builtin_writes", False))
        self._bridge_instance = None  # timeout may have changed; rebuild lazily
        profile = kwargs.get("agent_identity")
        self._source = f"hermes:{profile}" if profile else "hermes"
        self._resolve_python()

    def system_prompt_block(self) -> str:
        text = self._run(["read", "--scope", "all"]) or ""
        # `read` always emits section headers (and "(empty)" for empty stores);
        # only inject when there is substantive core content.
        substantive = [
            s for s in (ln.strip() for ln in text.splitlines())
            if s and not (s.startswith("=====") and s.endswith("=====")) and s != "(empty)"
        ]
        if not substantive:
            return ""
        return "# Mnemosyne Shared Memory\n\n" + text.strip()

    def prefetch(self, query: str, *, session_id: str = "") -> str:
        query = (query or "").strip()
        if not query:
            return ""
        rows = self._run(
            ["search", query, "--format", "json", "--limit", str(self._recall_limit)],
            json_out=True,
        )
        if not rows:
            return ""
        lines = ["## Mnemosyne recall"]
        for row in rows:
            summary = (row.get("summary") or "").strip()
            if not summary:
                continue
            try:
                score = float(row.get("score", 0))
            except (TypeError, ValueError):
                score = 0.0
            mtype = row.get("type", "")
            lines.append(f"- [{score:.1f}] ({mtype}) {summary}")
        return "\n".join(lines) if len(lines) > 1 else ""

    def sync_turn(self, user_content: str, assistant_content: str, *,
                  session_id: str = "", messages=None) -> None:
        # writes go through the explicit `mnemosyne` tool, not auto-sync
        pass

    def get_tool_schemas(self) -> List[Dict[str, Any]]:
        return [MNEMOSYNE_TOOL]

    def handle_tool_call(self, tool_name: str, args: Dict[str, Any], **kwargs) -> str:
        if tool_name != "mnemosyne":
            return tool_error(f"Unknown tool: {tool_name}")
        action = args.get("action")
        if action == "search":
            query = (args.get("query") or "").strip()
            if not query:
                return tool_error("search requires 'query'")
            limit = int(args.get("limit", self._recall_limit))
            rows = self._run(["search", query, "--format", "json", "--limit", str(limit)], json_out=True)
            return json.dumps({"results": rows, "count": len(rows)})
        if action == "write":
            if not args.get("type") or args.get("importance") in (None, ""):
                return tool_error("write requires 'type' and 'importance'")
            cmd = ["write", "--force", "--source", self._source,
                   "--type", str(args["type"]),
                   "--importance", str(args["importance"]),
                   "--scope", str(args.get("scope", "global"))]
            for flag, key in (("--title", "title"), ("--content", "content"), ("--tags", "tags")):
                val = args.get(key)
                if val is not None and str(val) != "":
                    cmd += [flag, str(val)]
            out = self._run(cmd)
            if not out:
                return tool_error("write failed")
            return json.dumps({"status": "ok", "output": out.strip()})
        if action == "show":
            mid = args.get("id")
            if not mid:
                return tool_error("show requires 'id'")
            return json.dumps({"memory": self._run(["show", str(mid)])})
        if action == "link":
            i, j, rel = args.get("id"), args.get("id2"), args.get("rel")
            if not (i and j and rel):
                return tool_error("link requires 'id', 'id2', 'rel'")
            out = self._run(["link", str(i), str(j), "--rel", str(rel)])
            return json.dumps({"status": "ok", "output": out.strip()})
        if action == "graph":
            mid = args.get("id")
            if not mid:
                return tool_error("graph requires 'id'")
            return json.dumps({"graph": self._run(["graph", str(mid), "--format", "mermaid"])})
        return tool_error(f"Unknown action: {action}")

    def on_memory_write(self, action: str, target: str, content: str,
                        metadata: Optional[Dict[str, Any]] = None) -> None:
        if not self._mirror or action != "add" or not (content or "").strip():
            return
        mtype = "preference" if target == "user" else "codebase"
        self._run(["write", "--force", "--source", self._source, "--scope", "global",
                   "--type", mtype, "--importance", "60",
                   "--title", content[:60], "--content", content])

    def _distill_enabled(self) -> bool:
        """Check ``[distill].enabled`` via the resolved bridge python.

        No CLI subcommand exposes config values, so this shells out a small
        inline script through the same python used for every other bridge
        call (kept consistent with ``_python_has_mnemosyne``).
        """
        script = (
            "import json,sys;"
            "from mnemosyne.store import load_config;"
            "print(json.dumps(bool(load_config().get('distill',{}).get('enabled'))))"
        )
        payload = self._bridge().run_json(["-c", script], raw_python=True)
        return bool(payload)

    def on_session_end(self, messages: List[Dict[str, Any]]) -> None:
        """Auto-distill durable memories from the finished conversation.

        Gated on ``[distill].enabled`` in the shared Mnemosyne config (off by
        default). Bridges through the same ``mnemosyne distill --stdin --commit``
        CLI path used elsewhere in this provider, tagged ``source="hermes"``.
        """
        if not self._distill_enabled():
            return
        text = _messages_to_text(messages)
        if not text.strip():
            return
        self._run(["distill", "--stdin", "--commit", "--source", self._source],
                   input_text=text)

    def shutdown(self) -> None:
        pass


def _load_plugin_config() -> Dict[str, Any]:
    """Read ``plugins.mnemosyne`` from Hermes config.yaml (best-effort)."""
    try:
        import yaml  # Hermes venv has PyYAML
        from hermes_constants import get_hermes_home  # type: ignore
        cfg_path = get_hermes_home() / "config.yaml"
        if not cfg_path.exists():
            return {}
        with open(cfg_path, encoding="utf-8-sig") as fh:
            data = yaml.safe_load(fh) or {}
        return (data.get("plugins") or {}).get("mnemosyne") or {}
    except Exception:
        return {}


def register(ctx) -> None:
    """Hermes plugin entry point."""
    ctx.register_memory_provider(MnemosyneMemoryProvider(config=_load_plugin_config()))
