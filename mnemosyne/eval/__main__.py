"""Command line entry point for retrieval evaluation."""

from __future__ import annotations

import argparse
from pathlib import Path

from mnemosyne.eval import (
    format_metrics,
    legacy_tokenize,
    run_evaluation,
    run_longmemeval,
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="mnemosyne eval")
    subparsers = parser.add_subparsers(dest="eval_command", required=True)

    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--corpus", type=Path, default=_default_corpus())
    run_parser.add_argument("--longmemeval", action="store_true", help="Per-instance LongMemEval scoring")
    run_parser.add_argument("--by-type", action="store_true", help="Break recall down by question type")
    run_parser.add_argument(
        "--pipeline",
        choices=["bm25", "full"],
        default="bm25",
        help="Scoring backend; 'full' routes through mnemosyne.fusion.search (only with --longmemeval)",
    )

    convert_parser = subparsers.add_parser("convert")
    convert_parser.add_argument("dataset", choices=["longmemeval"])
    convert_parser.add_argument("--raw", type=Path, required=True)
    convert_parser.add_argument("--out", type=Path, required=True)
    convert_parser.add_argument("--max-instances", type=int, default=None)

    fetch_parser = subparsers.add_parser("fetch")
    fetch_parser.add_argument("dataset", choices=["longmemeval"])
    fetch_parser.add_argument("--variant", choices=["s", "m"], default="s")
    fetch_parser.add_argument("--dest", type=Path, default=Path("~/.cache/mnemosyne/longmemeval").expanduser())

    args = parser.parse_args(argv)
    if args.eval_command == "run":
        return cmd_run(args)
    if args.eval_command == "convert":
        return cmd_convert(args)
    if args.eval_command == "fetch":
        return cmd_fetch(args)
    return 1


def cmd_run(args: argparse.Namespace) -> int:
    if args.longmemeval:
        report = run_longmemeval(args.corpus, pipeline=args.pipeline)
        for k in (1, 5, 10):
            print(f"recall@{k}={report[f'recall@{k}']:.3f}  ", end="")
        print(f"MRR={report['MRR']:.3f}  queries={int(report['count'])}")
        if args.by_type:
            for qtype, metrics in sorted(report["by_type"].items()):
                print(f"  [{qtype or 'unknown'}] recall@5={metrics['recall@5']:.3f}")
        return 0
    legacy = run_evaluation(args.corpus, tokenizer=legacy_tokenize)
    current = run_evaluation(args.corpus)
    print(format_metrics("legacy-bm25", legacy))
    print(format_metrics("bm25-only", current))
    print(f"bigram recall@5 delta={current['recall@5'] - legacy['recall@5']:+.3f}")
    return 0


def cmd_convert(args: argparse.Namespace) -> int:
    from mnemosyne.eval.adapters.longmemeval import convert

    convert(args.raw, args.out, max_instances=args.max_instances)
    print(f"Converted {args.raw} -> {args.out}/seed_memories.jsonl + corpus.jsonl")
    return 0


def cmd_fetch(args: argparse.Namespace) -> int:
    import urllib.request

    args.dest.mkdir(parents=True, exist_ok=True)
    url = _LONGMEMEVAL_URLS[args.variant]
    target = args.dest / f"longmemeval_{args.variant}.json"
    print(f"Downloading {url} -> {target}")
    print("LongMemEval is released for research use; review its license at the source repo.")
    try:
        urllib.request.urlretrieve(url, target)
    except Exception as exc:  # noqa: BLE001 - surface a clear message, don't crash
        print(f"Download failed: {exc}")
        print("Download manually and use `mnemosyne eval convert longmemeval --raw <file>`.")
        return 1
    print(f"Saved {target}")
    return 0


# Official LongMemEval distribution URLs (placeholder until the real source is confirmed).
_LONGMEMEVAL_URLS = {
    "s": "https://example.invalid/longmemeval_s.json",
    "m": "https://example.invalid/longmemeval_m.json",
}


def _default_corpus() -> Path:
    return Path(__file__).with_name("default_corpus.jsonl")


if __name__ == "__main__":
    raise SystemExit(main())
