# NX00–NX02 execution, 2026-09-26

The supplied GPT-6 Pro review is advisory input, not evidence of execution. Its fixed baseline is `a0da375a872001b57cb4f259492961de6edc2de6` (v2.0.1). The initial checkout was clean at `d05f028`; fetching origin confirmed a0da375 and work continued on `codex/eval-sleep-continuation` from that commit. No reset, installation, push, tag or publication was performed. No real memory or host configuration was changed by the tests. No model assets were downloaded and no model/paid-service evaluations were invoked. The local C2C bridge was checked, but there was no saved workspace web conversation; this batch uses the supplied Pro review, not a new web Pro response.

## Baseline

`git archive HEAD` supplied the clean baseline to a temporary directory. `cargo test --locked --all-targets` with network disabled and isolated HOME, MNEMOSYNE_HOME and target directory passed 159 tests. Evidence: `evidence/baseline/{metadata.txt,cargo-test.log,cargo-test.exit,coverage.md}`. The coverage note describes the baseline; the subsequent Stop test now additionally asserts stored facts, cumulative source counts, source/session identities and replay invariance. Direct fail-safe session_end also reads its saved record.

## Implementation and corrections

- NX01: pure field scoring per evidence arm, exact refusal/normalization, independent invocation/protocol/quality statuses, schema-2 summary and nonzero failure exit. Default CI includes offline scoring; no live evaluations were added to CI. Reports retain bounded raw answers for offline rescoring.
- NX02: ordered output pages and deterministic receipts under the existing MutationPlan journal. Proposal publication, receipt, run manifest and input cursor commit together. A pending output run reserves input progress; later batches cannot bypass it. Invalid records are reported as blocked while safe records continue. Long-ID metadata pages are bounded before receipt generation. Existing v1 reports replay without rewriting their evidence and interrupted v1 cursor writes recover.
- Review caught and corrected old-report replay, cross-batch cursor bypass, invalid ID records and long-ID metadata overflow. Added crash injection at four journal boundaries and all four output materialization prefixes, concurrent retry, source/heat change, malformed/oversized input, invalid page/order/budget, symlink/hardlink, empty terminal, old-report replay and CLI→MCP continuation checks.
- Early checks found an outdated oversized-input expectation, a test fixture missing source_kind, and a test-only moved stdout borrow. These were corrected. The retained `baseline/native-only-focused` failure is an intermediate check, not the final result.

## Final validation

The final commands and exit codes are in `evidence/final/results.json`; complete stdout/stderr are beside it. `tests/native_acceptance.py` creates a temporary HOME/MNEMOSYNE_HOME, preserves installed Cargo/Rustup caches and runs offline. It includes fmt, clippy, default and all-features tests, release build, transport/HTTP mocks, retrieval matrix, three recall gates and source package/list/unpack/offline-build checks. The HTTP model test uses local mock endpoints, not external model services. Scoring has its own `evidence/scoring.txt` and remains network/credential free.

Source, fixture recipe and binary hashes are in MANIFEST.json. Temporary randomized unit-test stores were deleted; their exact bytes are not claimed reproducible from a corpus hash. The fixture-source hashes identify the corpus construction recipes instead. This is local macOS validation; remote Linux/macOS CI has not run for this unpushed branch. LongMemEval is the bundled sample, not the full public dataset. No live cross-host/model quality or performance claim is made.

Final result: all 14 native acceptance commands exited 0; default and all-features each passed 170 tests. The separate seven scoring tests passed (exit 0). All three bundled recall gates were 1.000. This does not include remote CI or real-model performance/quality.

## Recovery and remaining scope

No production state was mutated. To undo this change, revert the branch diff; do not install the unreviewed working build as a rollback step. Test stores are disposable. At runtime, retry the exact output page after interruption; pending journal recovery completes committed pages or removes uncommitted intent. Source changes require a fresh export at zero. Unknown external edits during recovery retain the journal and fail closed.

NX03 V2 performance corpus and NX04 real producer→consumer host evaluation remain NOT_RUN. The earlier native-evolution R11-A05 and R14-A05 historical PARTIAL statuses remain unchanged: this batch documents supported bounds and does not turn limited offline coverage into unrestricted or live acceptance.

## Web Pro advisory continuation

The existing signed-in Chrome conversation was resumed. GPT-6 Pro answered the 21:55 execution-summary question; see `PRO_FOLLOWUP.md` for URL, scope and limits. The response was advisory, not a review of local code through a connector.

The targeted boundary review reproduced stale-source rejection of committed retries and missing sleep data in snapshot exports. Exact v2 replay now checks saved store/request identity before live inventory, never reapplies reviewed proposals, and cannot change a newer cursor. Snapshots preserve completed receipts and explicitly reject active continuations/forks rather than silently lose resumability. Abandoned historical runs remain exportable after current work completes. Added aborting child processes at four durable boundaries, late S1/S2 replay, missing-page, approve/reject replay and snapshot tests. Source-level review found no additional integrity regression. Repeated interruption *inside* recovery, injected disk I/O errors and real power-loss durability remain unverified.

The retained `evidence/final/` is the earlier 170-test checkpoint. New Rust gates in `evidence/pro-followup/` pass 181 tests per default/all-features configuration; Python runner changes after those gates are rechecked separately without rerunning unchanged Rust suites. Final source/binary/evidence hashes live in the top-level MANIFEST; the earlier manifest was preserved under `evidence/final/MANIFEST.json`.

NX03 now constructs and validates V2-current and V2-evolution corpora through formal writes, revisions, support, relations, checkpoints and proposals. Actual snapshot hash and generator-bound recipe hash are distinct. Every timed sample clones its seed and uses isolated environment; index absence is not an OS-cold-cache claim. Native Stop must add persisted V2 content, prep must contain the task action, and mutation queries must find the revised record. Inclusive spans are not summed across phases. Five samples are descriptive, not reliable tail estimates. No kernel/retrieval optimization or model download was made.

NX04 runs deterministic producer/consumer processes through CLI→MCP and Stop→CLI. Stop persists the actual service fact, not an unrelated setup record. Controller independently checks native persistence; consumer reads through native interfaces and never receives producer output/transcript/oracle. Fixed assertions share NX01 strict JSON parsing and its execution/protocol/quality distinction; the original fixed three-arm scorer is not relabeled as a generic handoff scorer. Dirty-worktree cases expose a real Git commit through an isolated git-only PATH. The controller owns worker process groups; nested native calls remain in that group, and timeouts cannot kill the caller's unrelated group. The descendant-cleanup regression exercises this nested path. Temporary directories/environment are test input isolation, not an OS sandbox.

Final V2 performance execution: both profiles at 100 and 1000 records, six scenarios each, five samples per scenario, all PASS. Release command records and actual corpus hashes are in `evidence/pro-followup/{perf-commands,v2-current,v2-evolution}.json`; descriptive medians are summarized in PRO_FOLLOWUP.md. 10000 and model lanes remain NOT_RUN.

## User scope correction

The user clarified that Mnemosyne is for personal use and does not need 10000-record evaluation. Retain the already verified 100/1000 V2 baselines; remove larger-scale performance testing from outstanding requirements. NX03-A04 passes the revised scope, with the original advisory wording preserved for traceability. No runtime limit or implementation was changed, and no code tests were rerun for this documentation-only correction.
