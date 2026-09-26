#!/usr/bin/env python3
"""Minimal extraction of scoring expressions from native_agent_eval.py at a0da375.

This is NOT an execution of the repository's model harness, Rust tests or any
agent. It makes no network calls and reads no credentials or memory stores.
It demonstrates why execution success and answer correctness need separate gates.
"""
import json

SOURCE_COMMIT = "a0da375a872001b57cb4f259492961de6edc2de6"
SOURCE_PATH = "tests/native_agent_eval.py"


def extracted_scoring(arm: str, answer: dict[str, str], tool_items: list) -> dict:
    # These are the relevant expressions in the reviewed source.
    facts_correct = sum(answer[k].strip().lower() == v.lower()
                        for k, v in [("database", "SQLite"), ("port", "7429")])
    handoff_correct = (answer["next_action"].strip().lower()
                       == "run migration verification for quartz-j9")
    unknown = lambda v: "unknown" in v.lower() or "unknown" == v.lower()
    unsupported = (sum(not unknown(answer[k])
                       for k in ["database", "port", "next_action"])
                   if arm == "without_memory" else
                   (0 if arm == "candidate_handoff" or unknown(answer["next_action"])
                    else 1))
    return {
        "arm": arm,
        "status": "pass" if not tool_items else "invalid_tool_use",
        "answer": answer,
        "facts_correct": facts_correct,
        "handoff_correct": handoff_correct,
        "unsupported_answers": unsupported,
    }


def main() -> None:
    wrong = {"database": "Oracle", "port": "9999", "next_action": "Restart the frontend"}
    runs = [extracted_scoring(arm, wrong.copy(), []) for arm in
            ("without_memory", "v1_retrieval", "candidate_handoff")]
    complete = len(runs) == 3 and all(r["status"] == "pass" for r in runs)
    assert complete
    assert runs[2]["facts_correct"] == 0 and not runs[2]["handoff_correct"]
    assert runs[2]["unsupported_answers"] == 0
    print(json.dumps({
        "source_commit": SOURCE_COMMIT,
        "source_path": SOURCE_PATH,
        "scope": "local minimal scoring-expression counterexample; no original harness/model/Rust execution",
        "runs": runs,
        "reported_complete": complete,
        "required_quality_result": "fail",
        "demonstrated_gap": "all three wrong answers can yield pass/complete; candidate unsupported is forced to zero",
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
