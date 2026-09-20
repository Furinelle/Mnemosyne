# R08 — read-only system-time content, explicit applicability

Historical queries use the existing verified immutable Markdown images and
manifest, never current retrieval indexes. They acquire an already-existing store
lock without recovery, observation or lock creation. Pending mutation journals
stop the read. Snapshot hashes are rechecked before returning content. Source
provenance is the append-only ledger prefix whose count/hash was recorded with
that version, excluding later SUPPORT events.

The API exposes history listing, explicit revision show, and lexical search at a
specified time. Half-open windows select the last revision at equal timestamps.
A backwards system timestamp is an explicit error, not silently reordered data.
Date-only input is UTC midnight; RFC3339 preserves its offset for the inclusive
expiry calendar-day rule, with both day and offset echoed in output. This answers
what the library recorded, not when a real-world fact became true.

Coverage is limited to known records. Pre-adoption, missing histories, observed
external-edit gaps and current content inconsistent with its latest observation
are explicit partial/unknown coverage. No current body is substituted. Future
records, current links, vector caches and usage heat cannot affect returned
historical content. Ranking is deterministic lexical overlap over selected
snapshots, without a claim to reproduce an old retrieval run's ranking.

Applicability is a separate current-worktree assessment, including on historical
results. Only explicit repo-wide scope, exact full commit/tree IDs on clean
worktrees, or exact repository-relative file hashes are accepted. An observed
commit/branch alone proves nothing. Unsafe paths, dirty/unknown Git state and
missing evidence return unknown. Reads of explicitly scoped records carry these
results into budgeted context warnings; applicability changes invalidate delivery
dedup even when the memory itself is unchanged.

Git checks use bounded output, a cleared environment, no optional locks, and
disabled filesystem-monitor/untracked-cache features. Effective repository filter
configuration (including includes) and gitlinks produce unknown before status,
so configured clean/process programs and submodule status are not executed.
Explicit related-path reads
are bounded independently of initial metadata. No repository hook is needed to
verify scope. Local work and tests remain isolated; no live configuration or
package release is part of this increment.
