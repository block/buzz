# Storage accounting

`buzz-admin storage-snapshot` lists S3, calculates physical and per-community
logical usage, and atomically replaces the completed snapshot in PostgreSQL.
The relay reads that snapshot and exposes the existing storage gauges on its
metrics endpoint for collection by Datadog or another metrics collector.

The same transaction appends one `storage_accounting_history` row per
community (`completed_at`, `community_id`, `logical_bytes`,
`logical_objects`, `code_sha`), so usage over time is queryable directly from
the relay database. History is append-only, is never read on the serving
path, and rows always correspond to a snapshot that actually published — a
worker whose lease session died can write neither. `community_id` is
provenance only: community deletion severs it to NULL instead of deleting
fleet accounting evidence.

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
