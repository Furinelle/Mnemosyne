# R09–R15 candidate decisions (2026-09-20)

- Reconciliation compares explicit subject/environment/attribute/cardinality and
  values; a canonical key also includes content so complementary/conflicting
  records can append. Provenance identity owns replay/SUPPORT. No model judge or
  similarity threshold authorizes semantic replacement.
- Same-store proposals reuse the R07 journal. A content hash is both the replay ID
  and review confirmation. Generation immediately records pending; approval audit
  and applied state commit with all endpoint revisions. There is no crash window
  with an approved flag and partially applied facts. Undo is a compensating
  revision and checks later endpoint/dependent changes. Cross-store proposals are
  refused, rather than inventing a second distributed transaction protocol.
- MCP body rewrites and semantic links create proposals. Approval remains trusted
  CLI; the host must control access to it. An agent with arbitrary shell access
  has the same authority as its OS user, so a tool annotation/hash is not security
  authentication. Existing unrelated links/non-body metadata remain supported.
- Daily decay is opt-in, UTC calendar based, non-negative and saturating. Initial
  activation does not charge a record's age. Daily heat cannot deprecate facts;
  legacy per-run retains its previous lifecycle projection. Pinned records keep
  low-frequency facts available, but do not bypass expiry or supersession.
- Sleep is an explicit offline/host export-import workflow. It scans bounded
  canonical/checkpoint snapshots, returns paginated metadata, and writes no fact
  automatically. Snapshot changes require cursor restart, with content-addressed
  proposals preventing duplicate replay. Hard byte/count caps fail explicitly;
  ordinary pagination has partial/next_cursor. Reports do not contain source
  bodies or transcripts. External endpoint invocation remains optional and absent.
- Views are optional generated source indexes. References identify revisions;
  changed/inactive sources stale the view. They are outside default retrieval and
  context, carry zero new evidence, and cannot overwrite manual core/edited views.
  Semantic synthesis/procedure generation is not implied by offline indexing.
- Snapshots are standard directories, bounded and allowlisted. Config/model/cache/
  transcript files are excluded; detected credentials refuse creation. Restoring
  validates every hash and canonical/history/proposal identity before publishing
  into a missing target with native no-clobber rename on macOS/Linux. Legacy
  per-memory locks are omitted only when creating. Derived search caches rebuild
  after publication because index rows contain final absolute paths.
- Restore preserves identity for an offline replacement; fork assigns a new ID,
  remaps structured references and invalidates copied pending approvals. Neither
  mode merges online stores. Duplicate selected identities are detected.
- Baseline package version stays 1.0.0 in the uncommitted development tree. The
  candidate changes public Rust struct construction and semantic MCP behaviors;
  the release must choose a breaking crate version before publication. Building
  is not installing or releasing. macOS gates, protocol fixtures, HTTP mocks,
  real model evaluation and actual host sessions are separate evidence levels.

- The fixed-model synthetic consumer experiment uses fresh Luna max CLI sessions,
  native baseline search and candidate `prep --task-id`, and reports exact fact/
  next-action recovery. It found a checkpoint display truncation bug; checkpoint
  summaries now remain complete or are omitted by the whole-bundle budget.
  Producer data are synthetic and the single task is not a general benchmark.
