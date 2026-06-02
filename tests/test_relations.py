from __future__ import annotations

import unittest

from mnemosyne.store import DEFAULT_CONFIG, load_config
from tests.helpers import isolated_workspace


class RelationTests(unittest.TestCase):
    def test_predefined_weight_and_unknown_fallback(self) -> None:
        from mnemosyne.relations import weight

        self.assertEqual(0.7, weight("refines"))
        self.assertEqual(0.5, weight("legacy-custom"))

    def test_weight_override_wins(self) -> None:
        from mnemosyne.relations import weight

        self.assertEqual(0.85, weight("refines", {"refines": 0.85}))

    def test_reverse_relation(self) -> None:
        from mnemosyne.relations import reverse

        self.assertEqual("causes", reverse("caused_by"))
        self.assertEqual("related", reverse("related"))
        self.assertIsNone(reverse("legacy-custom"))

    def test_symmetric_demoting_and_warning_flags(self) -> None:
        from mnemosyne.relations import is_demoting, is_symmetric, warns

        self.assertTrue(is_symmetric("contradicts"))
        self.assertTrue(warns("contradicts"))
        self.assertTrue(is_demoting("supersedes"))
        self.assertFalse(is_symmetric("refines"))

    def test_default_config_includes_v02_sections(self) -> None:
        with isolated_workspace():
            config = load_config()

        self.assertFalse(config["embedding"]["enabled"])
        self.assertFalse(config["rerank"]["enabled"])
        self.assertTrue(config["fusion"]["link_expansion"])
        self.assertFalse(config["relations"]["allow_custom"])
        self.assertEqual(5, config["mcp"]["default_search_limit"])
        self.assertIn("embedding", DEFAULT_CONFIG)


if __name__ == "__main__":
    unittest.main()
