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

- **0045 workflow revision binding: retained, validation scan avoided.** Nullable
  `definition_event_id` on workflows and runs binds new runs to
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
- **Supersession bookkeeping removed.** No events-column addition or events
  migration remains for this feature. Replacement and explicit deletion both
  revoke a captured definition for pending wake admission and continuation.
  Signed revision IDs are retained for exact association; historical deleted
  content is not an alternative source of authority.

Main already owns 0043 (push gateway profile) and 0044 (NIP-FI ledger removal).
Revision binding follows them as 0045. These PRs are unmerged. Disposable
fixtures which applied earlier draft migration numbers/checksums must not be
reused as an upgrade source. If an operator applied a draft elsewhere, reconcile
that ledger explicitly before upgrading. There are no down migrations.

## Operational cost and rollout

Runtime migration remains opt-in via `BUZZ_AUTO_MIGRATE`. The runner sets
`lock_timeout = 0` and `statement_timeout = 0` before taking its session-scoped
schema/destruction advisory lock; SQLx applies each migration file transactionally.
The revision migration supplies no timeout. Thus both advisory and
relation-lock waits are unbounded in normal auto-migrate startup. Metadata-only
is **not lock-free or guaranteed low latency**: a long-running transaction can
block DDL, whose queued lock can delay subsequent reads/writes. Operators should
schedule controlled schema application with this locking behavior in mind;
manual sessions can set explicit timeouts, and a failed file rolls back for retry.
No production duration estimate or live database operation is claimed.

PostgreSQL 17 remains the documented VISION/architecture/docker-compose contract,
so CI still uses 17; this feature no longer needs `SET EXPRESSION` support.
Deploy schema before new binaries. Mixed older writers clear projection
provenance, including equal-value semantic updates. NULL is unknown provenance,
not permission to infer a historical signature. Ordinary legacy execution is
not retroactively disabled, but signed continuation/wake reads fail closed.
Older readers do not implement the new 44620 recipient gate: introducing wake
producers into a mixed-reader deployment requires rollout coordination; unchanged
search indexing does not supply that authorization.

## Pending authority, not running-work cancellation

A wake retains its exact run/definition/message IDs. The authenticated authority
read requires that captured revision still match the workflow projection and
that its signed event be live. Edit or explicit deletion rejects old pending
wakes; it never rerenders their visible message with a new definition. A fresh
run/wake under the new revision works. Manual captured-definition loading and
stored approval resumes use the same current/live rule. The ordinary engine
still reports `approval_not_supported`; this is not a new approval product.

Transient authority fetch failures retry/replay the original wake and perform a
fresh authority lookup; 403/404 and invalid bundles remain terminal. The
admission decision uses the relay's current-pointer and live-event reads. These
are separate datastore reads, not a serializable snapshot or an atomic fence
through network delivery, local queuing, or arbitrary subsequent agent actions.
An edit/delete after the relevant read cannot retract the bundle already read by
that request or cancel running work. Signature verification alone proves
provenance, not perpetual freshness. No universal revocation/cancellation
guarantee is made.

## Regression evidence

- `postgres_workflow_revision_binding`: populated legacy tables retain heap
  files; checks stay unvalidated yet reject invalid new/updated IDs; legacy NULL,
  operational-update preservation and equal-value old-writer invalidation.
- `workflow_search_postgres_tests`: ordinary sink/replay, real indexed empty and
  malformed wakes, shared FTS candidates, actual HTTP/WS search responses,
  positive/NOT-only queries, explicit normal kinds, known-ID bypass attempts,
  recipient denial and revoked membership while a public control still returns.
- Existing unrelated FTS privacy and wake/count tests remain in place.
  Captured authority regressions reject both replacement and explicit deletion;
  the approval continuation regression rejects stale A rather than executing B.
- A bounded external runtime experiment drives the actual outer `buzz-acp` with
  signed sink-persisted wakes, a real HTTP/WS router and database, and a recording
  ACP child: unchanged revision returns 200/one prompt; edit or signed deletion
  before authority reads returns 404/zero prompts; 503 followed by deletion
  before the retry's authority read returns 404/zero prompts. The quiet window
  is five seconds. This starts at run/sink persistence, not manual-trigger UI,
  and proves neither indefinite exactly-once delivery nor cancellation after
  a returned bundle. Migration evidence is separate from this runtime proof.
