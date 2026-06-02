from __future__ import annotations

import unittest


class TokenizerTests(unittest.TestCase):
    def test_cjk_runs_are_split_into_bigrams(self) -> None:
        from mnemosyne.tokenizer import tokenize

        self.assertEqual(["调试", "试认", "认证"], tokenize("调试认证"))

    def test_single_cjk_character_is_preserved(self) -> None:
        from mnemosyne.tokenizer import tokenize

        self.assertEqual(["图"], tokenize("图"))

    def test_mixed_cjk_and_ascii_tokens(self) -> None:
        from mnemosyne.tokenizer import tokenize

        self.assertEqual(["oauth2", "认证", "证失", "失败", "api_v2"], tokenize("OAuth2认证失败 API_v2"))

    def test_punctuation_and_emoji_are_separators(self) -> None:
        from mnemosyne.tokenizer import tokenize

        self.assertEqual(["认证", "失败"], tokenize("认证，失败🙂"))

    def test_empty_input_has_no_tokens(self) -> None:
        from mnemosyne.tokenizer import tokenize

        self.assertEqual([], tokenize(""))

    def test_search_module_keeps_public_tokenize_alias(self) -> None:
        from mnemosyne.search import tokenize

        self.assertEqual(["认证"], tokenize("认证"))


if __name__ == "__main__":
    unittest.main()
