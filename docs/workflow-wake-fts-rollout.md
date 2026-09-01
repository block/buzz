# Workflow revision and wake database rollout (PostgreSQL 17)

## Decision: use ordinary event storage and the existing access boundary

There is **no wake-specific FTS migration or generated-expression change**.
Kind 44620 is a signed, durable, empty-content hint stored/replayed as an ordinary
event. Index membership is not permission to read it. We do not rewrite event
history, add projection-maintenance triggers, or create another event store.
Existing storage exclusions for unrelated private kinds are unchanged.

`buzz-search/src/query.rs` returns community-scoped candidate IDs and ranking
metadata, not snippets or aggregate totals. Its only production consumers are
WS NIP-50 REQ (`handlers/req.rs`) and HTTP `/query` (`api/bridge.rs`). Both hydrate
through scoped event reads, match the original NIP-01 filter (including IDs and
tags), and call `event_visible_to_reader` before delivery. Wakes require canonical
shape, the reader's recipient tag, and current channel membership from the DB;
open-channel readability alone is insufficient. Known-ID/kindless filters do not
bypass result authorization. COUNT uses the same per-event gate for wakes even
when `#p` is pinned to self; it does not expose FTS candidate counts. COUNT is not
a separate FTS search-total endpoint. Client snippets are derived from returned
authorized events, not raw indexed content.

Normal CLI/mobile message search selects kinds 9, 40002, 45001, 45003; profile
search uses kind 0. The ordered fresh-install search allowlist already excludes
wakes. The desired-state/brownfield broader policy may index them. NOT-only
queries can match an empty vector, but that is still just a candidate: current
recipient/membership checks decide visibility. Malformed nonempty wakes are
rejected by the same read gate. A currently authorized recipient may receive a
canonical wake through an explicit search, just as through ordinary replay.
There is no requirement that it have physical SQL NULL in the index.

Search pagination is bounded and post-filtering can underfill a page. These
paths do not promise constant-time execution or elimination of every statistical
pagination/timing side channel. Direct database operators are already trusted
with the underlying signed events; raw SQL access is not a tenant API.

## Migrations assessed separately

- **0043 workflow revision binding: retained, validation changed.** Nullable
  `definition_event_id` on workflows and runs is necessary to bind new runs to
  their exact signed definition, not a guessed current/historical revision.
  Adding the columns without defaults is metadata-only. Length-32 CHECKs are
  added `NOT VALID` to avoid historical validation scans under ACCESS EXCLUSIVE.
  PostgreSQL still checks every subsequent INSERT/UPDATE. All pre-existing rows
  read NULL because the columns are new. The catalog deliberately remains
  unvalidated: **no deferred validation job or backfill is planned**. Fresh
  desired-state schema creates validated checks on empty tables.
  The existing semantic-column BEFORE UPDATE trigger is retained to clear
  revision provenance for mixed older writers, even equal-value updates; new
  signed-event writers rebind within their locked transaction. This is an
  authority guard, not FTS projection maintenance.
- **Former 0044 wake FTS: removed.** No whole-events-tree inspection, lock,
  generated-expression rewrite, or history/index rebuild for search policy.
  Unknown custom FTS expressions need no wake-specific normalization.
- **Former 0045 superseded authority: retained as 0044.** A boolean with
  `NOT NULL DEFAULT false` uses PostgreSQL's fast-default metadata addition.
  `deleted_at` alone cannot distinguish replacement from explicit revocation.
  Positive replacement marks the bit; explicit deletion clears it, including
  on previously superseded rows. `get_workflow_revision` accepts live or
  positively superseded definitions, never unknown historical deletions.
  No historical provenance inference is introduced. This small metadata change
  still acquires ACCESS EXCLUSIVE locks on the events parent/partitions, but
  does not rewrite their heaps or indexes.

The PRs were open and unmerged at reassessment, with main ending at 0042. These
migration files have not been shipped by this stack. Disposable databases that
ran the superseded draft migration sequence must be recreated, not silently
reused with different SQLx checksums. If an operator has applied a draft outside
that recorded state, stop and reconcile its migration ledger explicitly before
upgrading. There are no down migrations.

## Operational cost and rollout

Runtime migration remains opt-in via `BUZZ_AUTO_MIGRATE`. The runner sets
`lock_timeout = 0` and `statement_timeout = 0` before taking its session-scoped
schema/destruction advisory lock; SQLx applies each migration file transactionally.
Neither retained migration supplies its own timeout. Thus both advisory and
relation-lock waits are unbounded in normal auto-migrate startup. Metadata-only
is **not lock-free or guaranteed low latency**: a long-running transaction can
block DDL, whose queued lock can delay subsequent reads/writes. Operators should
schedule controlled schema application with this locking behavior in mind;
manual sessions can set explicit timeouts, and a failed file rolls back for retry.
No production duration estimate or live database operation is claimed.

PostgreSQL 17 remains the documented VISION/architecture/docker-compose contract,
so CI still uses 17; this feature no longer needs `SET EXPRESSION` support.
Deploy schema before new binaries. Mixed older binaries may clear provenance or
fail closed on replacements they cannot positively identify, not invent legacy
revision authority. Signed wake persistence, captured-definition validation,
endpoint authorization, ACP verification/admission and identity/retry lifecycles
are unchanged by this database redesign.

## Regression evidence

- `postgres_workflow_revision_binding`: populated legacy tables retain heap
  files; checks stay unvalidated yet reject invalid new/updated IDs; legacy NULL,
  operational-update preservation and equal-value old-writer invalidation.
- `workflow_search_postgres_tests`: ordinary sink/replay, real indexed empty and
  malformed wakes, shared FTS candidates, actual HTTP/WS search responses,
  positive/NOT-only queries, explicit normal kinds, known-ID bypass attempts,
  recipient denial and revoked membership while a public control still returns.
- Existing unrelated FTS privacy tests and deletion-aware wake/count/approval
  tests remain in place. Previous ACP/sink/combined execution evidence applies
  to unchanged authority code, not as a claim of testing the new migration DDL.
