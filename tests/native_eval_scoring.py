"""Pure, offline scoring for the synthetic Quartz-J9 handoff experiment."""

import base64
import json
import re
import unicodedata


SCHEMA_VERSION = 2
MAX_RAW_ANSWER_BYTES = 8192
FIELDS = ("database", "port", "next_action")
REQUIRED_ARMS = ("without_memory", "v1_retrieval", "candidate_handoff")
UNKNOWN = "unknown"
FACT = "Quartz-J9 uses SQLite on port 7429."
ACTION = "Run migration verification for Quartz-J9"
ORACLE = {
    "without_memory": {
        "database": (UNKNOWN, ()), "port": (UNKNOWN, ()),
        "next_action": (UNKNOWN, ()),
    },
    "v1_retrieval": {
        "database": ("SQLite", (FACT,)), "port": ("7429", (FACT,)),
        "next_action": (UNKNOWN, ()),
    },
    "candidate_handoff": {
        "database": ("SQLite", (FACT,)), "port": ("7429", (FACT,)),
        "next_action": (ACTION, (ACTION,)),
    },
}
# Explicitly allow the refusal wording in the preserved 2026-09-12 evidence.
UNKNOWN_ALIASES = {"unknown", "unknown — no context supplied"}
TOOL_TYPES = {"command_execution", "mcp_tool_call", "web_search", "web_search_call",
              "function_call", "tool_call", "file_change", "image_generation",
              "dynamic_tool_call"}


def normalize(value):
    return " ".join(unicodedata.normalize("NFKC", value).casefold().split())


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def score_arm(arm, context, raw_answer, events=(), invocation_status="completed",
              context_budget_ok=True, output_word_limit=120, event_stream_valid=True):
    """Score only evidence actually delivered to this arm; never invoke a model."""
    if arm not in ORACLE:
        raise ValueError(f"unknown arm: {arm}")
    row = {"arm": arm, "invocation_status": invocation_status,
           "protocol_status": "not_checked", "quality_status": "not_scored",
           "facts_correct": 0, "fact_count": 2, "handoff_correct": False,
           "unsupported_answers": 0, "false_refusals": 0, "field_results": {}}
    if invocation_status != "completed":
        return row
    if raw_answer is None:
        row["protocol_status"] = "missing_output"
        return row
    if not isinstance(raw_answer, (str, bytes)):
        row["protocol_status"] = "bad_json"
        return row
    encoded = raw_answer.encode() if isinstance(raw_answer, str) else raw_answer
    bounded = encoded[:MAX_RAW_ANSWER_BYTES]
    if len(encoded) > MAX_RAW_ANSWER_BYTES:
        row["raw_answer_base64"] = base64.b64encode(bounded).decode("ascii")
        row["raw_answer_truncated"] = True
        row["protocol_status"] = "budget_exceeded"
        return row
    try:
        raw_text = bounded.decode("utf-8")
    except UnicodeError:
        row["raw_answer_base64"] = base64.b64encode(bounded).decode("ascii")
        row["protocol_status"] = "bad_encoding"
        return row
    row["raw_answer"] = raw_text
    try:
        answer = json.loads(raw_text, object_pairs_hook=unique_object)
    except (TypeError, ValueError):
        row["protocol_status"] = "bad_json"
        return row
    if not isinstance(answer, dict) or set(answer) != set(FIELDS) or any(
        not isinstance(answer[key], str) for key in FIELDS
    ):
        row["protocol_status"] = "bad_schema"
        return row
    row["answer"] = answer
    if not event_stream_valid or any(not isinstance(event, dict) for event in events):
        row["protocol_status"] = "invalid_event_stream"
        return row
    if any(event.get("item", {}).get("type") in TOOL_TYPES or
           str(event.get("item", {}).get("type", "")).endswith("_tool_call")
           for event in events if isinstance(event.get("item", {}), dict)):
        row["protocol_status"] = "invalid_tool_use"
        return row
    answer_words = sum(len(re.findall(r"\S+", value)) for value in answer.values())
    if not context_budget_ok or answer_words > output_word_limit:
        row["protocol_status"] = "budget_exceeded"
        return row
    row["protocol_status"] = "valid"
    context_text = normalize(context)
    for field, (expected, evidence) in ORACLE[arm].items():
        value = normalize(answer[field])
        refused = value in UNKNOWN_ALIASES
        supported = all(normalize(marker) in context_text for marker in evidence)
        if expected == UNKNOWN:
            correct = refused
            if not correct:
                row["unsupported_answers"] += 1
        else:
            correct = supported and value == normalize(expected)
            if refused and supported:
                row["false_refusals"] += 1
            elif not correct and not refused:
                row["unsupported_answers"] += 1
        row["field_results"][field] = {
            "correct": correct, "evidence_present": supported,
            "expected": expected, "evidence": list(evidence),
        }
        if field in ("database", "port") and correct:
            row["facts_correct"] += 1
        if field == "next_action" and expected != UNKNOWN:
            row["handoff_correct"] = correct
    row["quality_status"] = "passed" if all(
        result["correct"] for result in row["field_results"].values()
    ) and row["unsupported_answers"] == row["false_refusals"] == 0 else "failed"
    return row


def summarize(rows):
    """Missing, duplicate, failed, or unscored arms cannot satisfy acceptance."""
    by_arm = {row.get("arm") for row in rows}
    complete = (len(rows) == len(REQUIRED_ARMS)
                and by_arm == set(REQUIRED_ARMS)
                and all(row.get("invocation_status") == "completed" for row in rows))
    accepted = complete and all(
        row.get("protocol_status") == "valid" and row.get("quality_status") == "passed"
        for row in rows
    )
    return {"schema_version": SCHEMA_VERSION, "required_arms": list(REQUIRED_ARMS),
            "completed_arms": [row["arm"] for row in rows
                               if row.get("invocation_status") == "completed"],
            "execution_complete": complete, "acceptance_passed": accepted}
