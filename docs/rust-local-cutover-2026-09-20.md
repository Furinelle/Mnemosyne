# Local native evolution cutover — 2026-09-20

Installed the locally validated development candidate; no GitHub publication.
CLI version remains 1.0.0, so identify this build by SHA-256:
`2429f2434d262f98cbe5b5126e337575fc6ecb133e648d1ac4bfa2260cb2cf3f`.

- Stable local executable now resolves to native/2429f2434d262f98/mnemosyne.
- Codex and Claude settings unchanged. Grok inspect confirms enabled inherited
  Claude hooks. Actual configured commands passed isolated tests: Codex 3,
  Claude 4, Grok 4 (Stop reentrancy/no-op path).
- Both Antigravity MCP configurations now use the stable local executable.
  Old MCP children were stopped; refreshed UI shows both servers enabled with
  eight tools each, and two replacement processes use the stable executable.
  Plugin toggle reported a server-name error; direct config update and refresh
  successfully reloaded it.
- Both actual MCP config commands passed initialize/list/core/search/context-v2
  calls from `/` with an explicit project path. No model chat was submitted.
- Global and this project's stores upgraded to schema 2, minimum writer 3.
  Doctor: 269 global and 21 project memories, canonical status ready.
  Original Markdown unchanged: 271 global files and 23 project files.
- Real cached ONNX embedding, vector retrieval, incremental backfill and
  cross-encoder reranking passed using temporary stores. Live vector settings
  were preserved (doctor reports disabled).
- Complete stores copied under their locks before upgrade. Private backups,
  file hashes, exact config originals, validation and rollback procedure are in
  `~/.local/share/mnemosyne/backups/evolution-20260920-171840/`.
  The old executable/library remain installed. Downgrade requires matching
  store rollback; switching only the executable is unsafe.

Other project stores were not migrated. No synthetic memories were written to
real stores. Future host calls use this build; existing conversation context is
not retroactively replaced.
