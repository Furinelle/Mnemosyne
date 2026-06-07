"""Command line entry point for retrieval evaluation."""

from __future__ import annotations

import argparse
from pathlib import Path

from mnemosyne.eval import format_metrics, legacy_tokenize, run_evaluation


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="mnemosyne eval")
    subparsers = parser.add_subparsers(dest="eval_command", required=True)
    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--corpus", type=Path, default=_default_corpus())
    args = parser.parse_args(argv)
    return cmd_run(args.corpus)


def cmd_run(corpus: Path) -> int:
    legacy = run_evaluation(corpus, tokenizer=legacy_tokenize)
    current = run_evaluation(corpus)
    print(format_metrics("legacy-bm25", legacy))
    print(format_metrics("bm25-only", current))
    print(f"bigram recall@5 delta={current['recall@5'] - legacy['recall@5']:+.3f}")
    return 0


def _default_corpus() -> Path:
    return Path(__file__).with_name("default_corpus.jsonl")


if __name__ == "__main__":
    raise SystemExit(main())
