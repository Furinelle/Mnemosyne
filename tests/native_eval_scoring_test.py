#!/usr/bin/env python3
"""Offline regression for the Quartz-J9 scoring counterexample."""

import json
import unittest

from native_eval_scoring import (ACTION, FACT, MAX_RAW_ANSWER_BYTES,
                                 REQUIRED_ARMS, score_arm, summarize)


def answer(database, port, next_action):
    return json.dumps({"database": database, "port": port, "next_action": next_action})


class ScoringTest(unittest.TestCase):
    def test_supplied_counterexample_fails_all_arms(self):
        wrong = answer("Oracle", "9999", "Restart the frontend")
        contexts = ("No context supplied.", FACT, FACT + " " + ACTION)
        rows = [score_arm(arm, context, wrong) for arm, context in
                zip(REQUIRED_ARMS, contexts)]
        self.assertTrue(summarize(rows)["execution_complete"])
        self.assertFalse(summarize(rows)["acceptance_passed"])
        self.assertEqual([row["unsupported_answers"] for row in rows], [3, 3, 3])
        self.assertEqual(rows[2]["quality_status"], "failed")
        self.assertEqual(rows[2]["facts_correct"], 0)
        self.assertFalse(rows[2]["handoff_correct"])

    def test_correct_answers_and_exact_normalization(self):
        rows = [
            score_arm("without_memory", "No context supplied.",
                      answer(" UNKNOWN ", "unknown", "unknown — no context supplied")),
            score_arm("v1_retrieval", FACT,
                      answer(" sqlite ", " 7429 ", "unknown")),
            score_arm("candidate_handoff", FACT + " " + ACTION,
                      answer("SQLite", "7429", ACTION)),
        ]
        self.assertTrue(summarize(rows)["acceptance_passed"])
        self.assertEqual(rows[1]["facts_correct"], 2)
        self.assertTrue(rows[2]["handoff_correct"])

    def test_candidate_action_wrong_or_refused(self):
        for next_action in ("Restart the frontend", "unknown"):
            row = score_arm("candidate_handoff", FACT + " " + ACTION,
                            answer("SQLite", "7429", next_action))
            self.assertEqual(row["facts_correct"], 2)
            self.assertEqual(row["quality_status"], "failed")
            self.assertFalse(row["handoff_correct"])

    def test_evidence_controls_support_even_if_hidden_answer_is_guessed(self):
        self.assertEqual(score_arm("without_memory", "No context supplied.",
                                   answer("SQLite", "7429", ACTION))["unsupported_answers"], 3)
        row = score_arm("candidate_handoff", FACT, answer("SQLite", "7429", ACTION))
        self.assertEqual(row["quality_status"], "failed")
        self.assertFalse(row["field_results"]["next_action"]["evidence_present"])
        row = score_arm("v1_retrieval", "No context supplied.",
                        answer("SQLite", "7429", "unknown"))
        self.assertEqual(row["unsupported_answers"], 2)

    def test_false_refusal_and_mixed_unknown_assertion(self):
        row = score_arm("v1_retrieval", FACT,
                        answer("unknown", "7429", "unknown, but restart frontend"))
        self.assertEqual(row["false_refusals"], 1)
        self.assertEqual(row["unsupported_answers"], 1)
        self.assertEqual(row["quality_status"], "failed")
        row = score_arm("v1_retrieval", FACT,
                        answer("SQLite.", "7429", "unknowned"))
        self.assertEqual(row["quality_status"], "failed")
        self.assertEqual(row["unsupported_answers"], 2)

    def test_invalid_protocol_and_budget(self):
        valid = answer("SQLite", "7429", ACTION)
        cases = [
            ("{", {}, "bad_json"),
            ('{"database":"SQLite","database":"Oracle","port":"7429","next_action":"unknown"}',
             {}, "bad_json"),
            (json.dumps({"database": "SQLite", "port": "7429"}), {}, "bad_schema"),
            (json.dumps({"database": "SQLite", "port": 7429, "next_action": ACTION}), {}, "bad_schema"),
            (valid, {"events": [{"item": {"type": "command_execution"}}]}, "invalid_tool_use"),
            (valid, {"context_budget_ok": False}, "budget_exceeded"),
            (valid, {"output_word_limit": 2}, "budget_exceeded"),
            (valid, {"event_stream_valid": False}, "invalid_event_stream"),
            (b"\xff", {}, "bad_encoding"),
            ("x" * (MAX_RAW_ANSWER_BYTES + 1), {}, "budget_exceeded"),
            (None, {}, "missing_output"),
        ]
        for raw, kwargs, expected in cases:
            row = score_arm("candidate_handoff", FACT + " " + ACTION, raw, **kwargs)
            self.assertEqual(row["protocol_status"], expected)
            self.assertEqual(row["quality_status"], "not_scored")
            if raw is not None:
                self.assertTrue("raw_answer" in row or "raw_answer_base64" in row)
        self.assertEqual(score_arm("candidate_handoff", FACT + " " + ACTION, "{")["raw_answer"], "{")

    def test_execution_errors_and_missing_arm(self):
        for status in ("timeout", "failed", "not_run"):
            row = score_arm("candidate_handoff", FACT + " " + ACTION, None,
                            invocation_status=status)
            self.assertEqual(row["invocation_status"], status)
            self.assertEqual(row["quality_status"], "not_scored")
        correct = score_arm("candidate_handoff", FACT + " " + ACTION,
                            answer("SQLite", "7429", ACTION))
        self.assertFalse(summarize([correct])["execution_complete"])
        self.assertFalse(summarize([correct])["acceptance_passed"])
        self.assertFalse(summarize([correct, correct, correct])["acceptance_passed"])


if __name__ == "__main__":
    unittest.main()
