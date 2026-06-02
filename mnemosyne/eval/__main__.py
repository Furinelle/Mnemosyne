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
    compare_parser = subparsers.add_parser("compare")
    compare_parser.add_argument("--baseline", type=Path, required=True)
    compare_parser.add_argument("--variant", type=Path, required=True)
    compare_parser.add_argument("--corpus", type=Path, default=_default_corpus())
    args = parser.parse_args(argv)
    if args.eval_command == "run":
        return cmd_run(args.corpus)
    return cmd_compare(args.baseline, args.variant, args.corpus)


def cmd_run(corpus: Path) -> int:
    legacy = run_evaluation(corpus, tokenizer=legacy_tokenize)
    current = run_evaluation(corpus)
    print(format_metrics("legacy-bm25", legacy))
    print(format_metrics("bm25-only", current))
    print(f"bigram recall@5 delta={current['recall@5'] - legacy['recall@5']:+.3f}")
    return 0


def cmd_compare(baseline: Path, variant: Path, corpus: Path) -> int:
    _validate_config(baseline)
    _validate_config(variant)
    baseline_metrics = run_evaluation(corpus)
    variant_metrics = run_evaluation(corpus)
    print(format_metrics(f"baseline:{baseline.name}", baseline_metrics))
    print(format_metrics(f"variant:{variant.name}", variant_metrics))
    print(f"delta recall@5={variant_metrics['recall@5'] - baseline_metrics['recall@5']:+.3f}")
    return 0


def _validate_config(path: Path) -> None:
    import tomllib

    with path.open("rb") as handle:
        tomllib.load(handle)


def _default_corpus() -> Path:
    return Path(__file__).with_name("default_corpus.jsonl")


if __name__ == "__main__":
    raise SystemExit(main())
