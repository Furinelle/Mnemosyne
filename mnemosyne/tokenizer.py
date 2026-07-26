"""Language-light tokenization with CJK bigram support."""

from __future__ import annotations

import re


# Scripts that need character-level matching because they are written without
# spaces: kana, CJK ideographs (incl. extension A and the compatibility block),
# and Hangul syllables. Anything omitted here is silently unsearchable, since
# characters outside TOKEN_RE are treated as separators.
_DENSE_RANGES = (
    "぀-ヿ"  # hiragana + katakana
    "㐀-䶿"  # CJK unified ideographs extension A
    "一-鿿"  # CJK unified ideographs
    "豈-﫿"  # CJK compatibility ideographs
    "가-힯"  # Hangul syllables
)
_DENSE_RE = re.compile(f"[{_DENSE_RANGES}]")
# The Latin range keeps accented letters (café, naïve) in one token instead of
# splitting on the accent and dropping it.
TOKEN_RE = re.compile(f"[{_DENSE_RANGES}]+|[a-z0-9_À-ɏ]+", re.IGNORECASE)


def tokenize(text: str) -> list[str]:
    """Split dense-script runs into bigrams and keep word runs intact."""
    tokens: list[str] = []
    for chunk, dense in script_runs(text):
        if dense:
            if len(chunk) == 1:
                tokens.append(chunk)
            else:
                tokens.extend(chunk[index : index + 2] for index in range(len(chunk) - 1))
        else:
            tokens.append(chunk)
    return tokens


def script_runs(text: str) -> list[tuple[str, bool]]:
    """Lowercased token runs paired with a "dense script" flag.

    Callers that must respect a downstream tokenizer's own segmentation — e.g.
    building an FTS5 trigram MATCH expression — need these unsplit runs rather
    than the bigrams `tokenize` produces.
    """
    return [
        (match.group(0), _is_dense(match.group(0)[0]))
        for match in TOKEN_RE.finditer(text.lower())
    ]


def _is_dense(character: str) -> bool:
    return _DENSE_RE.match(character) is not None
