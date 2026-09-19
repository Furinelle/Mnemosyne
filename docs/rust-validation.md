# Rust migration acceptance — 2026-09-19

This is the initial candidate acceptance record. The later actual local cutover,
additional fixes, and real-model checks are in [rust-local-cutover.md](rust-local-cutover.md).

Baseline: Python v0.8.0, commit `5a3e89eb59f65b6b6641774de83013cbf39dae39`.
Branch: `codex/rust-migration`. Local platform: macOS ARM64, Rust 1.95.0.

| Check | Actual result |
| --- | --- |
| `cargo test --locked --all-targets` | 27 library + 7 CLI integration tests passed; exit 0 |
| `cargo fmt --all -- --check` | Passed; exit 0 |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed, including optional ONNX compilation; exit 0 |
| `cargo run --quiet --locked -- eval run --min-recall 0.95` | 50 queries, BM25 recall@5 1.000, MRR 1.000; exit 0 |
| Same eval with `--pipeline full` | 50 queries, FTS5/fusion recall@5 1.000, MRR 1.000; exit 0 |
| Eval `--longmemeval --pipeline full --min-recall 0.95` | Built-in **2-case sample**, recall@5 1.000; exit 0 |
| `cargo build --locked --release` | Passed; native executable about 8.4 MiB; exit 0 |
| `cargo package --locked --allow-dirty` | Archive created and extracted package compiled successfully; exit 0; not published |
| `python3 tests/native_transport_smoke.py target/release/mnemosyne` | Actual SSE handshake, malformed UTF-8 request rejection without service exit, ping passed |
| `python3 tests/native_hermes_smoke.py target/release/mnemosyne` | Thin plugin called the actual Rust binary for core, write, recall and session-end distillation; passed |
| Isolated `.venv/bin/python -m pytest tests/ -q -p no:cacheprovider` | Existing Python baseline: 310 tests and 6 subtests passed; exit 0 |
| `uv tool run --from ruff ruff check mnemosyne tests adapters/hermes` | Passed; exit 0 |

Native and Python suites were run with temporary HOME/MNEMOSYNE_HOME. Model
evaluation lanes were explicitly disabled in the fixed corpus gates; these
numbers are retrieval regression results, not generated-answer benchmark scores.

## Real-store copies

The actual global and project stores were locked while copying Markdown into
temporary directories. Endpoint/model configuration was excluded. Rust reindex
was executed against those copies, never the original stores:

- Global: Python could read 269 records; Rust indexed 269.
- Project: Python could read 21 records; Rust indexed 21.
- Both SQLite indexes returned `integrity_check = ok`.
- Every copied Markdown SHA-256 was unchanged after indexing.
- Temporary copies were removed automatically. No original memory was rewritten.

## Limits and next acceptance boundary

- No production host configuration was changed, no binary installed over the
  existing Python command, and no push/release performed.
- Linux/macOS CI configuration was added; hosted CI was not run in this session.
- Actual ONNX model execution and actual remote provider requests were not run.
  Optional ONNX code compiled; tokenizer/vector-cache tests are separate evidence.
- Hermes was tested through its real thin adapter and the native executable in
  isolation, not inside a running Hermes session. Other live hosts were not switched.
- Python import APIs are not native bindings. Existing Python code remains a
  reference until callers are migrated. Deliberate compatibility restrictions
  are listed in [rust-migration.md](rust-migration.md).
- The subsequent provenance/history/checkpoint/proposal roadmap has not started.
