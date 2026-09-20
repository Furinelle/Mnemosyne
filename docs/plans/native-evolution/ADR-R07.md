# R07 — one recovery protocol, opt-in semantic history

The existing `.relations-operation.json` owns v1, v2 and v3 recovery. V3 adds
optional before/after images (absence and deletion), immutable history images,
history manifests and the narrowly scoped evidence JSON path. It does not create
a second transaction engine. Same-store plans use a decision bit in the synced
journal. Cross-store plans use the existing trusted coordinator/participant digest
protocol; mixed writer-3/legacy participants are rejected. Normal completion
removes the decision only after both participants finish. Interrupted decisions
are retained conservatively, never removed by age.

Schema remains 2, but the minimum writer protocol becomes 3 through explicit
`store-upgrade --commit`. The same store UUID survives a writer-2 upgrade. Preview
and reads never implicitly raise the marker. New history is enabled only on
writer-3 stores. This is a storage protocol increment, not a package/release bump.
Old binaries must not operate on these upgraded stores; rollback restores the
whole pre-upgrade copy, not just the marker or a disposable index.

`revisions::plan_updates` separates semantic expectations from exact disk CAS.
The caller captures revision/hash before expensive computation and computes
outside the lock. The commit path checks expected semantics, merges unchanged
heat fields from current state, and records exact before/after images. All plan
targets are validated before intent, and again before the durable decision.
Immutable snapshot collisions never become owned staged files. Recovery first
checks every target and refuses unexpected external content without partial
repair. Advisory locking still requires cooperating writers; direct file edits
must follow the documented store-lock convention.

Knowledge changes create immutable numbered Markdown images and manifest entries.
No-op and heat-only updates do not advance semantic history. Source-event support
changes a ledger hash/count in Markdown and commits its sidecar in the same plan.
Archive moves add a semantic archived_at marker while keeping superseded/deprecated
status intact. Reads return memory, revision and provenance within one store lock.

The first observed old record is an adopted snapshot with a coverage start. An
external semantic edit is a new observed snapshot and an explicit unknown gap;
filesystem mtime is never the system-time receipt. Current Markdown is not
normalized by observation. History manifests determine committed visibility;
uncommitted staged images can be removed during rollback. Neither caches nor the
pending journal are the historical database.

First implementation deliberately validates full history under the existing store
lock. Incremental verification belongs in R14 if measurements justify it. No
network replication, CRDT, generic distributed transaction service or historical
query UI is introduced here.
