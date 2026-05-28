from __future__ import annotations

import unittest

from mnemosyne.hooks._common import format_for_injection


class InjectionFormattingTests(unittest.TestCase):
    def test_format_for_injection_respects_approx_token_budget(self) -> None:
        results = [
            {
                "id": "memory-1",
                "scope": "project",
                "type": "codebase",
                "tags": ["alpha"],
                "summary": "A" * 240,
                "score": 3.0,
                "strength": 90,
            },
            {
                "id": "memory-2",
                "scope": "project",
                "type": "pitfall",
                "tags": ["beta"],
                "summary": "B" * 240,
                "score": 2.0,
                "strength": 40,
            },
        ]

        text = format_for_injection(results, max_tokens=80)

        self.assertIn("memory-1", text)
        self.assertNotIn("memory-2", text)
        self.assertLessEqual(len(text) // 4, 80)


if __name__ == "__main__":
    unittest.main()

