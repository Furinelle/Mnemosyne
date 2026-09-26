# NX00 baseline

Baseline: clean HEAD `a0da375a872001b57cb4f259492961de6edc2de6`; Rust 1.95.0. `cargo test --locked --all-targets` passed 22 targets with 159 passed, 0 failed. It ran offline from a `git archive` snapshot with temporary `HOME`, `MNEMOSYNE_HOME`, and `CARGO_TARGET_DIR`; effective `CARGO_HOME` and `RUSTUP_HOME` were preserved. No model tests or external services were invoked. See `metadata.txt`, `cargo-test.log`, and `cargo-test.exit` for hashes, toolchain, full output, and status.

Existing relevant coverage:

- `tests/native_only.rs:565` upgrades a fixture store, commits ingest findings, checks duplicate replay and independent-event support, then drives positive Stop hooks for Codex, Claude, Grok, and Antigravity. It checks the `auto-saved` result and that replaying each hook is suppressed. `tests/native_only.rs:723` checks V2 preview as new, duplicate, and supported; checks no preview evidence/history writes; checks evidence remains searchable; and confirms committed support. Additional preview tests cover legacy Markdown and unadopted V2 sidecars.
- `tests/reconciliation.rs:32` checks duplicate/SKIP, independent SUPPORT, conflicting-value CONTRADICT, and a complementary CREATE. `tests/reconciliation.rs:88` checks REFINE, changed-environment CONTEXTUALIZE, multivalued CREATE, and `automatic_replacement: false`.
- `tests/provenance_v2.rs:315` checks support bound to a corrected semantic revision, rejects stale old content, replays duplicates, and preserves source revision history through snapshot/restore and tamper detection. Tests at lines 460 and 566 cover refine/undo support and unknown pre-binding source revisions.
- `tests/sleep.rs:213` pages 45 large records through the 4 MiB byte budget, checks unique coverage and cursor progress, and runs rules page by page. Other sleep tests cover stale cursors, oversized inputs, failed imports, and proposals that must not mutate facts.

Coverage gaps for this evaluation:

- Stop tests do not read back Stop-created Markdown/provenance or assert the saved event/evidence count; the direct `session_end` case checks success and `auto-saved` output but not stored state or replay.
- Reconciliation checks classification and write outcomes in temporary stores; it does not cover a live environment or any external caller.
- The large-page test proves continuation beyond the first page and enforces each page's byte ceiling, but does not assert an exact page count or an exact boundary-sized page. It is a synthetic local-store test, not a real agent/model session.

The log contains the whole-suite baseline; this note describes existing tests only and makes no source changes.
