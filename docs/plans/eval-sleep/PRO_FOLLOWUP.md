# Web GPT-6 Pro follow-up — 2026-09-26

Conversation: https://chatgpt.com/c/6aab4dc8-0ab8-83e9-88c1-2a28ad6e0071

The existing Chrome conversation showed Pro selected and gpt-6-pro for the response. The request was submitted at 21:55 (Asia/Singapore); the completed response reported 7m30s. This was an advisory consultation from a concise execution summary. No workspace connector was connected; Pro did not inspect this uncommitted diff or certify its tests. Advice is a reference, not independent evidence of local correctness.

## Adopted scope

1. First verify committed-response loss, stale/late requests, missing output pages, concurrent replay/content conflicts, reviewed proposal replay, and snapshot/restore/fork handling. Reuse the transaction engine.
2. Build legal V2-current and V2-evolution corpora through native interfaces; validate actual persisted data and identities. Measure 100/1,000 separately from the old legacy baseline. Construction time is separate, each sample starts with equivalent state, inclusive timings are not summed. The user subsequently removed 10,000-record evaluation from scope for personal use; it is not planned follow-up work.
3. Exercise independent deterministic producer/consumer processes with CLI→MCP and Stop-fixture→CLI paths. Check unresolved failure, revised facts/source bindings, dirty worktree revalidation and project isolation. Test the runner's own failure paths. This is offline native contract coverage, not model-quality or live-host acceptance.
4. Stop extension on lost/duplicated effects, advanced incomplete cursors, external-edit overwrite, oracle leakage or false-positive scoring. Do not add retrieval architecture, run paid models, touch real stores or publish as part of this phase.

## Local follow-up findings

- Confirmed: `finish_page` checked live source before reading committed receipt. Reordered exact receipt replay before live-source validation; new receipts bind store identity. New/uncommitted pages still require current snapshot. Replay never reapplies proposal state or rewinds progress.
- Confirmed: portable snapshot allowlist omitted sleep receipts/progress. Completed receipts must survive restore; unfinished current continuations explicitly refuse snapshot because inventory identity includes filesystem metadata. Fork with sleep receipts explicitly refuses instead of reusing original identities.
- Added abrupt child-process abort checks at four durable transaction boundaries, including after committed materialization before a response. These supplement existing returned-error/materialization-prefix tests; they do not prove power-loss durability or injected fsync/ENOSPC/EIO handling.

Final execution evidence is recorded separately under `evidence/pro-followup/`. Earlier `evidence/final/` remains the NX00–NX02 first-batch checkpoint, not validation of later edits. Acceptance statuses must retain unexecuted external/model/large-scale cases.

## Boundary evidence scope

- B01: abrupt child termination at intent, snapshot, commit-decision and completed-materialization checkpoints; exact retry checks receipt identity, proposal state/count and cursor. This includes committed output before caller response.
- B02: existing recovery tests cover each materialization prefix and preserve unknown external edits. At the first follow-up checkpoint, repeated termination inside recovery was NOT_RUN. Release-local validation now includes commit abort, recovery after one object abort, recovery after two more objects abort, then exact recovery/replay with every journal after-image checked. Injected fsync/ENOSPC/EIO and power-loss durability remain NOT_RUN.
- B03–B06: missing page cannot be replaced by duplicate/final; old committed replay survives S2 while old uncommitted page fails; same-page concurrent threads and content/total changes are checked; approved/rejected proposal bytes survive replay.
- B07: unfinished current snapshot refuses explicitly; completed snapshots preserve receipt bytes and retry; abandoned historical run does not permanently block snapshot; fork refuses sleep identity reuse.
- B08: outputs do not enter source inventory; access/semantic changes invalidate new work under the documented strict metadata contract. No claim that access-only changes are ignored.
- Retained resource limits remain explicit. Empty nonfinal pages are finite because total is immutable and capped at 1,000. There is no receipt-cleanup feature; no cleanup behavior is claimed.

## Measured V2 baseline (release, no model)

Each cell is the median of five independent process samples, in milliseconds. Construction is outside query timing. These are descriptive local baselines, not a speedup claim or stable tail estimate.

| Profile / records | Build seconds | Cache absent | Warm search | Revised query | Prep task | File hook | Stop new |
|---|---:|---:|---:|---:|---:|---:|---:|
| v2-current / 100 | 8.70 | 136.12 | 117.79 | 46.58 | 26.80 | 45.42 | 93.20 |
| v2-current / 1000 | 146.18 | 775.39 | 732.64 | 280.46 | 183.54 | 309.70 | 246.14 |
| v2-evolution / 100 | 14.08 | 138.00 | 121.94 | 49.61 | 28.29 | 45.94 | 95.09 |
| v2-evolution / 1000 | 347.02 | 901.57 | 829.51 | 417.24 | 220.50 | 395.25 | 288.81 |

All 4 corpora × 6 paths × 5 samples passed persisted-data/correctness checks. Evolution-1000 contains 1,201 sources, 1,601 revisions, 100 superseded facts, 5 checkpoints and 5 pending proposals. Stop replay timings remain in the JSON reports. The slow construction uses existing formal APIs; no storage or search optimization was introduced.

Reproduce from the repository root with `python3 tests/native_perf.py target/release/mnemosyne --profile v2-evolution --records 100 --records 1000 --samples 5 --command-timeout 120 --overall-timeout 600 --commit <base+uncommitted> --output <report.json>`; repeat with `v2-current`. Run `python3 tests/native_handoff_eval.py target/debug/mnemosyne --output <handoff.json>` for the eight offline contract cases.

## Release consultation and final local candidate

The same signed-in Chrome conversation completed a second GPT-6 Pro response (2m49s). Pro recommended conditional release for personal use after the exact candidate passes existing remote CI and its actual packaged artifacts pass version/basic-operation checks. Real models/hosts and storage fault injection are non-blocking explicitly unverified scope; 10,000 records remain outside user scope. This was summary advice, not workspace-connected code review.

The targeted repeated-abort regression was independently completed while Pro answered. `evidence/release-local/` records all 18 acceptance commands passing for 2.0.2, including 181 default and 181 all-feature Rust tests. CI now unpacks each platform's archive and runs its executable version and eight native handoff cases. Remote results belong to the exact release commit and will be linked in release notes after completion.

The 120 performance samples remain bound to the pre-version-bump binary and source manifest in `evidence/pro-followup/MANIFEST.json`; they were not rerun or relabeled as packaged release measurements. Subsequent changes are release metadata/docs and test-only recovery fault points, not a new runtime performance implementation. No live store, model assets, credentials or private memory were included.
