from __future__ import annotations

import unittest

from mnemosyne.codex import parse_findings


class CodexIngestTests(unittest.TestCase):
    def test_parse_findings_accepts_repeated_header_blocks(self) -> None:
        text = """Codex finished.

**新发现:**
- type: codebase
- importance: 70
- title: First finding
- tags: one
- content: |
    first content

**新发现:**
- type: pitfall
- importance: 80
- title: Second finding
- tags: two
- content: |
    second content
"""

        findings = parse_findings(text)

        self.assertEqual(["First finding", "Second finding"], [item.title for item in findings])
        self.assertEqual(["codebase", "pitfall"], [item.type for item in findings])


class EvidenceTests(unittest.TestCase):
    def test_parse_findings_reads_evidence_field(self) -> None:
        text = (
            "**新发现:**\n"
            "- type: pitfall\n"
            "- importance: 70\n"
            "- title: cache bug\n"
            "- tags: cache\n"
            "- evidence: turn 3, assistant\n"
            "- content: |\n"
            "    stale cache caused the failure\n"
        )

        findings = parse_findings(text)

        self.assertEqual(1, len(findings))
        self.assertEqual("turn 3, assistant", findings[0].evidence)

    def test_write_finding_persists_evidence(self) -> None:
        from mnemosyne.codex import Finding, write_finding
        from mnemosyne.store import find_memory, stores_for_scope
        from tests.helpers import isolated_workspace

        with isolated_workspace():
            from mnemosyne.cli import main

            self.assertEqual(0, main(["init"]))
            finding = Finding(
                type="pitfall", importance=70, title="cache bug",
                tags=["cache"], content="stale cache caused the failure",
                evidence="turn 3, assistant",
            )

            memory_id = write_finding(finding, "test")

            found = find_memory(memory_id, stores_for_scope("all"))
            self.assertIsNotNone(found)
            self.assertEqual("turn 3, assistant", found[2].extra.get("evidence"))


if __name__ == "__main__":
    unittest.main()

