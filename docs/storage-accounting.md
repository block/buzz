# Storage accounting

`buzz-admin storage-snapshot` lists S3, calculates physical and per-community
logical usage, and atomically replaces the completed snapshot in PostgreSQL.
The relay reads that snapshot and exposes the existing storage gauges on its
metrics endpoint for collection by Datadog or another metrics collector.

## Independent rollout

Deploy the snapshot-reading relay version through the usual release process.
It is ready before a worker exists: no database row means no storage gauges.
Add a scheduled worker to each environment when ready. Its first successful
publication becomes visible on the next leader usage tick, without restarting
the relay or changing its mode. The default tick interval is 300 seconds.

The snapshot table is part of the normal database schema, independent of worker
installation. A missing table or inaccessible database is an error, not evidence
that the worker is absent. Snapshot reads have a five-second budget, including
connection acquisition, and retry on the next usage tick. They do not run on
the relay startup path and never fall back to an S3 scan.

`BUZZ_STORAGE_METRICS` accepts:

| Value | Behavior |
|---|---|
| Unset, `external`, `snapshot`, `on` | Read completed worker snapshots. |
| `inline` | Legacy alias for the reader; logs a migration warning once. |
| `off` | Skip storage reads and metrics. |
| Anything else | Log a configuration error and disable storage metrics. |

Relay-local scans have been removed. Existing chart values that set `inline`
need no coordinated configuration change. Environments without a worker will
stop producing storage totals when this relay version arrives. The former
`BUZZ_STORAGE_SWEEP_*` settings no longer apply; size and bound the worker
instead. An intentional `off` override still requires an operator to enable it.

## Freshness and failures

The reader publishes the last completed totals together with
`buzz_storage_snapshot_age_seconds`. Re-reading an old row never resets its
completion time. Alert on age using a threshold appropriate for the worker's
schedule and allowed run time; for a daily worker, allow more than 24 hours.
`buzz_storage_snapshot_load_ok=1` means the row was read and decoded, not that
the worker's most recent attempt succeeded. Check Job failures separately.

A failed or invalid read logs an error, sets load health to zero, and retains
the last good totals with their original age. With no prior good snapshot,
only failed-load health is emitted. A successful read that finds no row clears
the cache and stops refreshing storage series. Existing exported series expire
through the metrics exporter's idle timeout; absence is never reported as zero
bytes. A valid empty snapshot can report zero bytes.

The reader preserves leader-only emission, community scope filtering, and
cleanup of series for renamed or removed communities. It does not need S3
listing permission. The worker still needs that permission and its database
configuration.

## Usage history feed

The same transaction that replaces the snapshot appends one
`storage_usage_history` row per community: every community in the snapshot,
plus an explicit zero for each serving community (not archived, not being
deleted) the snapshot omits, so a consumer keeping the latest value per
community never keeps a stale nonzero one. Each run also prunes rows older
than 400 days. A worker whose lock session died writes neither the snapshot
nor history.

An operator reads the history with an operator-signed NIP-98 request:

```text
GET /operator/storage-usage?after=<cursor>&limit=<1..1000, default 500>
```

```json
{
  "usage": [
    {
      "cursor": 4812,
      "measured_at": "2026-10-30T03:04:12Z",
      "community_host": "acme.example.com",
      "storage_bytes": 123456789,
      "storage_objects": 412
    }
  ],
  "next_cursor": 4812
}
```

- Rows come oldest first. `measured_at` is the run's completion time and is
  the same for every row of a run. `storage_bytes` and `storage_objects` are
  the logical per-community values described above: a blob bound to several
  communities counts in full for each.
- `after` defaults to `0`, the oldest retained row. Store `next_cursor` only
  after the page is durably recorded, and pass it back as `after`. An empty
  page means the consumer has caught up; `next_cursor` then echoes `after`.
- Cursors grow in commit order because runs serialize on the worker lock, so
  a consumer never skips a row that commits later with a smaller cursor.
- Archived or deleting communities that still hold bytes are included.
  `community_host` is the community's current host. Rows for a community with
  no community row, or whose community was deleted (deletion severs their
  provenance), are skipped, so a page can be shorter than `limit` without
  being the last.
- The relay keeps no delivery state and never calls out. A consumer that
  falls behind catches up on its next poll, as long as it polls within the
  retention window.
