"""Convert LongMemEval instances into Mnemosyne's seed + corpus format.

Each haystack session becomes one seed document id 'lme-<qid>-<sid>'. Each
question becomes an EvalItem whose expected_ids are the evidence sessions.
Retrieval is scored per-instance (the harness isolates by instance_id).
"""

from __future__ import annotations

import json
from pathlib import Path


def _session_text(session: list[dict]) -> str:
    return "\n".join(str(turn.get("content", "")) for turn in session)


def convert(raw_path: Path, out_dir: Path, *, max_instances: int | None = None) -> None:
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    instances = json.loads(Path(raw_path).read_text(encoding="utf-8"))
    if max_instances is not None:
        instances = instances[:max_instances]

    seed_path = out_dir / "seed_memories.jsonl"
    corpus_path = out_dir / "corpus.jsonl"
    with seed_path.open("w", encoding="utf-8") as seed_handle, corpus_path.open(
        "w", encoding="utf-8"
    ) as corpus_handle:
        for inst in instances:
            qid = str(inst["question_id"])
            session_ids = inst["haystack_session_ids"]
            sessions = inst["haystack_sessions"]
            dates = inst.get("haystack_dates", [""] * len(session_ids))
            for sid, session, sdate in zip(session_ids, sessions, dates):
                seed_handle.write(
                    json.dumps(
                        {
                            "id": f"lme-{qid}-{sid}",
                            "text": _session_text(session),
                            "instance_id": qid,
                            "session_date": sdate,
                        },
                        ensure_ascii=False,
                    )
                    + "\n"
                )
            expected = [f"lme-{qid}-{sid}" for sid in inst.get("answer_session_ids", [])]
            if not expected:
                # Abstention questions ("*_abs") have no evidence session. Scoring
                # them would count a fixed recall/MRR of 0 for every one of them,
                # making the aggregate incomparable to published numbers.
                continue
            corpus_handle.write(
                json.dumps(
                    {
                        "query": inst["question"],
                        "expected_ids": expected,
                        "paraphrase_of": "",
                        "notes": str(inst.get("answer", "")),
                        "instance_id": qid,
                        "question_type": str(inst.get("question_type", "")),
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
