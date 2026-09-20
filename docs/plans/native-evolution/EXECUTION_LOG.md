# Native evolution execution log

## M0 — R00–R03 and initial R14 baseline (2026-09-19)

Started from clean `d05f02853f42771ae9ca04d78b79739d01f1a76c` (Rust 1.0.0).
Work is on local branch `codex/native-evolution`; no version bump, commit,
push, tag, installation or host configuration changes were performed in this batch.
The imported plan's seven file hashes were verified before copying. Its original
static-review assertions are hypotheses; the results below are new local evidence.

### Baseline and contract (R00)

`evidence/baseline.json` records HEAD, toolchain/features and evaluation hashes.
The initial 44 native tests, fmt and clippy passed (exit 0); raw output is in
`evidence/baseline-*.txt`. The release-turn CI success for the fixed base is
historical evidence only, not a CI run for this uncommitted batch.
CLI help and eight-tool schemas were captured before modification. The new
`interface_contract` fixture checks shared CLI/MCP knowledge fields and native
aliases. CLI search credits usage after ranking, while MCP search is read-only;
subsequent score differences are explicitly asserted and documented rather than
claiming identical side effects. Interface/adapter docs now describe fs2 locks,
Rust cache names, recovery obligations and retired Python/Hermes boundaries.

### Reproduced defects and fixes

- R01: original source/lifecycle suite had 2 passes and 4 failures (exit 101).
  Shared write dedup now compares normalized source. Maintenance preserves
  superseded status and excludes it from core candidates; per-run decay remains.
  `evidence/r01-regression.txt` maps all six cases to commands and outcomes.
- R02: zero-vector fresh-cache regression failed (exit 101). Backfill validates
  cached vectors, repairs bad rows, uses per-batch transactions, and exposes
  counters. Query inference checks its profile again and degrades explicitly to
  lexical search on stale/corrupt/incompatible vectors. Fake/loopback tests cover
  body/config/resource races, invalid provider output and disabled-model behavior.
  See `evidence/r02-regression.txt` for A01–A06 and limitations.
- R03: original notification/envelope/lifecycle/UTF-8 cases failed. Requests now
  validate the JSON-RPC envelope; notification-shaped tool calls do not write;
  business errors use isError; initialization state is tracked per connection.
  Shared bounded input covers CLI stdin, stdio and SSE. Eight tools, aliases and
  project_path remain. See `evidence/r03-regression.txt` for protocol sources,
  exact tests and the explicitly retained legacy direct-call path.

### Reproducible final validation

Run from the repository:

```sh
python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/m0-final
```

The runner creates a temporary HOME, MNEMOSYNE_HOME and evaluation cwd, strips
ambient configuration/credentials, and keeps the configured Cargo/Rustup paths.
It runs fmt, all-feature clippy, default/all-feature tests, SSE, local HTTP model
smoke, the unchanged three recall gates, package whitelist validation, and an
offline build of the unpacked source package. Each command has a raw log and exit
code in `evidence/m0-final/results.json`. Final result: all 12 commands exited 0; default and all-feature suites each
passed 64 tests. The source archive rebuilt offline using cached dependencies.

The 50-query lexical/full corpus and 2-case LongMemEval smoke sample are separate.
No complete public benchmark or real-host/model conversation was run. Real ONNX,
paid endpoints and live host acceptance are NOT_RUN in this batch; previous
release evidence is not relabeled as current acceptance.

### R14 performance baseline (partial task)

`tests/native_perf.py` was run against an isolated release build of exactly
`d05f028`, with compiled ONNX support but all models disabled. The command is:

```sh
python3 tests/native_perf.py <baseline-release-binary> --commit d05f02853f42771ae9ca04d78b79739d01f1a76c --output docs/plans/native-evolution/evidence/perf-baseline.json
```

Raw samples, corpus and executable SHA-256, environment, p50/p95 and sample counts
are in that JSON. Sizes are 100/1,000/10,000 synthetic memories; five samples each
measure process startup, cold index, warm query, same-size single-file edit and
file_touch injection. These are process totals, not internal phase attribution.
No performance optimization or across-version speedup claim was made. Internal
phase profiles, model fingerprint timing, pending-recovery latency, lane ablation
and comparative cross-agent end-to-end evaluation remain NOT_RUN. R14 stays in
progress, rather than claiming its six acceptance items are fully complete.

### Compatibility, rollback and next batch

No canonical schema migration is required by M0. Additional source-specific
records remain ordinary Markdown and must not be deleted when rolling back.
Vectors remain disposable derived data; access heat does not change embedding
input identity. Error consumers must handle both JSON-RPC errors and isError tool
results. SSE rejects non-loopback binds; larger native stdin requires the documented
bounded environment override. JSON backfill prints counters and exits nonzero
when invalid/failed results occur.

This batch finishes M0 only. The next dependency-ready implementation is R04
(stable identity/provenance), then R05 and R06. R07–R13/R15 remain not started;
checkpoint/history/proposals/sleep are not claimed as implemented. The original
95-item backlog remains visible with explicit per-item status and evidence.

## M1 — R04–R06 (2026-09-20)

Continued the existing uncommitted M0 branch without replacing or resetting its
changes. R04, R05 and R06 are now locally implemented and accepted. R07–R13/R15
remain not started; R14 remains the explicitly partial baseline task. Package
version remains 1.0.0. No commit, push, release, installed binary, real memory
store, host configuration, model download or paid endpoint changed in M1.

### Implementation and compatibility

- R04 adds explicit schema-2 manifests, stable store/memory identity, source-event
  ledgers and deterministic idempotency/conflict results. Same-fact SUPPORT checks
  the current working Markdown before adding evidence, and rejects corrected,
  superseded, invalidated, expired or archived records. Bounded sidecars are synced
  before Markdown and repair interrupted initial publication under the store lock.
  Preview remains read-only even with pending work. Caller `verified` claims are
  downgraded; unknown legacy provenance remains unknown. See `ADR-M1.md`.
- R05 adds budgeted ContextBundle output shared by CLI/MCP/hooks, semantic revision
  and context-epoch delivery tracking, and explainable path matching. Titles,
  sources, warnings, core, hints and checkpoints consume the same estimated budget.
  Access heat does not cause repeated injection. Legacy output remains available.
- R06 adds native project checkpoints, revision CAS, explicit scoped-file/commit
  fingerprints, expiration and close semantics. Reports are always reported-only;
  planned commands are not executions. Checkpoints never execute commands or become
  durable knowledge automatically. An explicit matching task_id selects at most
  three active unexpired records; unreadable records produce a bounded warning.

Upgrading a real store is a separate operational decision: new legacy writers
refuse schema 2, and older binaries cannot enforce this future protocol. Existing
v1 auto-ingest paths require migration before an actual store upgrade. The test
suite explicitly upgrades temporary copies only and restores both upgraded and
pre-upgrade copies. Manifest, provenance sidecars and Markdown must be preserved
together; deleting just store.json is not rollback. V1 bare-ID read behavior remains
first-match; v2 accepts qualified IDs and rejects ambiguous unqualified IDs.

### Verification and evidence

```sh
cargo test --offline --test provenance_v2
cargo test --offline --test checkpoints
cargo test --offline --test m1_contract
python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/m1-final
```

The final shared gate completed all 12 commands with exit 0. Default and
all-feature suites each passed 80 tests. This includes six provenance tests,
five checkpoint tests and two integrated v1/v2/CLI/MCP/hook contract tests.
Fmt, all-feature Clippy with warnings denied, stdio/SSE and HTTP-model mocks,
the existing three recall gates, source package whitelist and unpacked offline
build all passed. Raw commands, outputs and source archive hash are retained in
`evidence/m1-final/`. The 50-query retrieval corpus and 2-case LongMemEval smoke
remain samples, not a full public benchmark.

`evidence/r04-regression.txt`, `r05-regression.txt` and `r06-regression.txt` map
all 19 M1 acceptance cases to concrete checks. The first integration lint run
reported an arithmetic spelling, a wide existing-style helper signature and an
unnecessary clone in a test; these were corrected before the final gate. New
contract tests initially needed expectation corrections for preview status and
MCP scalar wrapping; no preexisting product behavior was changed to satisfy those
incorrect expectations.

The 12 directed four-host handoff combinations are native protocol fixtures only.
No live Codex/Claude Code/Grok/Antigravity model conversation or live cutover was
performed. Git dirty-file acceptance uses a real temporary local commit; test
failures are actually executed in the temporary fixture and remain caller reports
when loaded. Injected clocks cover expiry without modifying the system clock.
The next dependency-ready task is R07, semantic revision and recoverable writes.

## R07 — semantic revision and recoverable mutation (2026-09-20)

R07 is locally implemented and accepted on the same uncommitted branch. M2 is not
complete: R08 and R09 remain not started, as do later tasks except R14's historical
partial baseline. No real-store migration, installation, commit, push or release
was performed. Package version remains 1.0.0; schema stays 2 and the explicitly
opted-in minimum writer protocol is now 3.

The existing relation recovery engine now accepts bounded v3 MutationPlans with
before/after images, create-absence checks and deletion for archive moves. The
same journal and trusted coordinator retain v1/v2 recovery. History images and
source sidecars commit with current Markdown; manifests expose committed history.
Prepared failures roll back staged snapshots, committed failures roll forward,
and unexpected external content retains the journal with RECOVERY_CONFLICT.
Historical versions remain ordinary immutable Markdown plus a versioned manifest,
independent of SQLite. See `ADR-R07.md` and the updated interface specification.

`revisions` defines semantic hashes, caller snapshots and a common planner. Body,
source/evidence, type, tags, links, lifecycle/expiry and archive changes advance
history; access heat does not. New corrections use `revise-v2` or the versioned MCP
write `revise` operation, with expected revision/hash. `show-v2` provides that
snapshot and reads provenance/current state under one lock. First observation of
an adopted record preserves its raw snapshot and coverage boundary; later external
edits record observation time and an explicit unknown gap, never invented past
bodies or mtime-derived commit times.

Source create/SUPPORT, corrections, same/cross-store relation changes,
consolidation and maintenance use the common path in upgraded stores. SUPPORT
persists a source ledger hash/count with its sidecar in the same operation. Archive
moves carry an archived_at marker while preserving superseded/deprecated status.
Both participants must support writer 3 for revision-aware cross-store writes;
reads and preview do not auto-upgrade a writer-2 store. Older binaries must be
stopped before any eventual live upgrade, and rollback restores the complete store
copy, not just its marker. The earlier M1 writer-2 evidence remains historical.

Validation:

```sh
python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r07-final
```

The runner now also includes the plan's release build with ONNX enabled. All 13
commands exited 0; default and all-feature tests each passed 101 tests. The suite
includes 15 relation tests, six revision tests and five new CLI/MCP integration
tests. Fmt, all-feature Clippy, native transport/model mocks, all three unchanged
retrieval gates, packaging and unpacked offline build passed. No real model assets
were loaded or downloaded. Raw evidence and the seven acceptance mappings are in
`evidence/r07-final/` and `evidence/r07-regression.txt`.

Final review corrected split-lock show results, missing archive history when
status did not change, mixed-writer cross-store plans, and preexisting snapshot
ownership on failed preparation. Each is covered by a focused regression. Initial
fixture argument/schema mistakes and one Clippy warning were corrected; no check
was skipped to achieve the final result. Tests do not claim live four-host sessions
or production-scale history performance. Next dependency-ready work is R08,
system-time history queries and conservative code applicability.


## R08 — system-time history and code applicability (2026-09-20)

R08 is locally implemented and its six acceptance cases passed. M2 remains
incomplete until R09. This increment did not migrate real stores, install a binary,
change host settings, commit, push or publish. Package version remains 1.0.0;
schema 2 and opt-in writer protocol 3 are unchanged.

CLI history, show --revision and search --as-of share the committed-history path
with the existing versioned MCP tools. Half-open system-time intervals, query-offset
expiry dates and explicit incomplete coverage prevent invented historical content.
Historical ranking uses snapshot text only; current vectors, graph edges and future
source events cannot supply content. Historical reads neither recover pending
operations nor change canonical files, usage, expiry or session delivery state.

Code applicability uses explicit repo-wide, exact commit/tree or bounded path-hash
claims. Missing evidence, dirty worktrees, local Git filters and gitlinks remain
unknown where required; observed commit/branch never imply ancestry applicability.
Applicability is evaluated against the current worktree, including for historical
snapshots, and feeds structured results and budgeted context warnings. The history
API does not reproduce past rankings or claim real-world fact validity intervals.
See ADR-R08.md and docs/interface.md for the contract and limits.

Validation:

```sh
python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/r08-final
```

All 13 gates exited 0. Default and all-feature suites each passed 115 tests,
including five historical cases, seven applicability cases and the expanded
CLI/MCP read-only contract. Fmt, Clippy, release build with ONNX, transport/model
mocks, the three existing retrieval gates, packaging and unpacked offline build
passed. This is isolated fixture acceptance, not live four-host/model acceptance
or a complete public LongMemEval benchmark.

Terra high implemented applicability; Luna max implemented bounded history tests.
The main agent integrated and reviewed the changes. Final review added effective
Git configuration inspection including included files, rejection of dangling
pending-journal symlinks, and isolated Git test environments. Initial lint issues
were corrected before the final gate. Raw results and acceptance mappings are in
evidence/r08-final/ and evidence/r08-regression.txt. Next dependency-ready work is
R09, reconciliation and the proposal ledger.


## R09–R15 — reviewed evolution and local candidate closure (2026-09-20)

All remaining implementation modules are present: conservative reconciliation and
reviewed proposals, repeatable daily maintenance, bounded sleep/host import,
derived source indexes, and safe snapshots/restore/fork. Shared CLI/MCP integration
keeps eight advertised tool names. Destructive MCP body/relation requests now
return pending proposals; trusted CLI approval checks the reviewed summary hash,
all endpoint revisions and undo dependencies. This behavior change is explicitly
recorded rather than presented as silently identical to older MCP semantics.
See ADR-M3.md and docs/interface.md for the scoped contracts.

Reconciliation and proposals share existing provenance and R07 mutation/history.
No second mutation engine or model client was introduced. Source identities govern
SUPPORT/replay, scoped value/content identities preserve conflicting/complementary
facts. Semantic links update both endpoints atomically and prevent replacement
cycles. The approval audit record commits with applied state; retry is idempotent.
Compensation preserves evidence and current usage and refuses later dependent edits.

Maintenance preserves per_run compatibility; per_day is opt-in and uses fixed UTC
calendar tests, activation initialization and saturating non-negative decay.
Pinned facts resist heat-driven archive, without bypassing expiry/supersession.
Per_day cannot turn low heat into a deprecated truth status. Sleep has bounded
canonical/checkpoint inputs, no transcript retention or automatic semantic writes,
and snapshot-bound pagination. Host import only creates proposals. Views are
traceable offline indexes outside default retrieval and context, with zero new
evidence and protection against overwriting manual edits/core.

Snapshots validate canonical/history/proposal identities, allowlisted paths,
SHA-256, file/total limits, symlinks/hardlinks and pending operations. Native
no-clobber publication protects existing destinations. Config/endpoints, models,
raw transcripts and caches are excluded. Fork remaps structured identity and
invalidates copied pending approvals. Restored CLI search caches rebuild at the
final path. The fixed baseline 1.0.0 upgrade exercise verified byte equality for
canonical records, history, evidence, checkpoints and proposals after restoration.
The installed local path reported 0.8.0, so it was not used as a supposed 1.0.0
baseline and was never changed. A clean fixed-commit 1.0.0 build supplied the test.

Validation:

```sh
python3 tests/native_acceptance.py --output docs/plans/native-evolution/evidence/final --old-binary target/baseline-v1/debug/mnemosyne
python3 tests/native_perf.py target/release/mnemosyne --output docs/plans/native-evolution/evidence/r14-perf-final.json --commit d05f02853f42771ae9ca04d78b79739d01f1a76c --samples 3
```

All 15 final gates exited 0. Default and all-feature suites each passed 144 tests.
The gate includes fmt, all-feature Clippy, release ONNX build, transport/model
mocks, requested/effective lane matrix, baseline upgrade/restore, the three existing
retrieval thresholds, package allowlist and unpacked offline compilation. The
LongMemEval check remains the bundled sample, not the full public benchmark.

Review found and corrected old per-memory .md.lock snapshot compatibility,
per-day low-heat status mutation, decay overflow/negative values, unsafe cursor
subtraction, and direct file-path context candidates bypassing expired/superseded
filters. Each affected path has a regression; final gates were repeated after
the lifecycle filter correction. No failing check was discarded.

Performance is not uniformly faster: on the 10k synthetic corpus the final three
samples measured p50 approximately 537 ms cold indexing, 251 ms warm query and
517 ms file hook. The added file-path matching path costs an extra scan; the
baseline file-hook mean was about 191 ms. These are process totals on this host,
not phase-level profiler attribution or a general speed claim. No content-hash
or external-edit checks were weakened to win the benchmark. Raw samples, binary
SHA and baseline comparison are retained in r14-perf-final.json.

Terra high handled maintenance, views, sleep and measurement tooling. Sol high
handled data-integrity-sensitive snapshots and proposal review. The main agent
integrated contracts, reconciliation, tests and final verification. No actual
store migration, host configuration edit, binary installation, commit, push, tag
or release occurred. The working tree still uses baseline package 1.0.0 and must
choose an appropriate breaking crate version before publication. Linux CI is
configured but not run remotely; live four-host UI sessions and real ONNX assets
remain separate NOT_RUN evidence, not implied by local fixture success.


### Fixed-model handoff comparison and final correction

The last R14 criterion was exercised with three fresh real Codex CLI consumers,
fixed `gpt-5.6-luna` / `max`, synthetic source data, an identical task and equal
prompt/output budgets. The no-memory arm correctly returned unknown; 1.0.0 native
retrieval recovered database and port; final candidate native `prep --task-id`
also recovered the exact checkpoint next action. All three had zero unsupported
answers. This measures a bounded handoff-recovery task with a synthetic producer,
not independent real producer/consumer applications or statistical task success.
Actual usage and hashes are saved in evidence/r14-agent-eval.json.

The initial harness read selected checkpoint fields directly. It was strengthened
to use the real `prep` output, which exposed a real failure: the general 120-char
memory summary cap truncated checkpoint next_action. Context now keeps complete
checkpoint summaries and relies on the existing whole-bundle budget to omit a
whole checkpoint when it cannot fit. A focused regression proves the next action
survives and a constrained budget does not deliver a misleading partial handoff.
The pre-fix model report is retained separately. Nine short CLI calls across the
initial harness, real-path diagnosis and final verification were made; there were
no real Mnemosyne model endpoint calls or user memory/transcript inputs.

The final native suite has 144 tests in each feature configuration and 15 gates.
All R00–R15 implementation/acceptance entries are locally closed within the
recorded scope. Remaining deployment conditions are real macOS/Linux host rollout,
optional real ONNX/public-corpus validation, choosing a breaking crate release
version and explicit publication; none is falsely labeled a performed operation.

Local binary candidate and SHA256SUMS are available under target/release-candidate/.
They are explicitly named unreleased. Source/test/asset/interface package bytes
were compared with the final workspace, and the source package was rebuilt offline
after documentation closure. No GitHub publication is implied by these artifacts.

## 2026-09-20 publication check

The local host cutover passed configured Codex/Claude/Grok hooks, both
Antigravity MCP configurations, real cached ONNX retrieval/reranking and
canonical Markdown preservation checks. See the dated local cutover record.

A bounded publication review found that snapshot restore/fork omitted the
derived MEMORY.md directory. Restore now regenerates it in the private staging
directory before publication using the existing index writer. The snapshot
restore/fork regression checks both outputs contain the restored memory ID.
Historical final-gate logs and binary hashes above describe the pre-fix build;
GitHub CI validates the final committed source separately.
