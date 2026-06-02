"""Language-light tokenization with CJK bigram support."""

from __future__ import annotations

import re


TOKEN_RE = re.compile(r"[a-z0-9_]+|[\u4e00-\u9fff]+", re.IGNORECASE)


def tokenize(text: str) -> list[str]:
    """Split CJK runs into bigrams and keep ASCII word runs intact."""
    tokens: list[str] = []
    for match in TOKEN_RE.finditer(text.lower()):
        chunk = match.group(0)
        if _is_cjk(chunk[0]):
            if len(chunk) == 1:
                tokens.append(chunk)
            else:
                tokens.extend(chunk[index : index + 2] for index in range(len(chunk) - 1))
        else:
            tokens.append(chunk)
    return tokens


def _is_cjk(character: str) -> bool:
    return "\u4e00" <= character <= "\u9fff"
