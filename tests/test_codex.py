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


if __name__ == "__main__":
    unittest.main()

