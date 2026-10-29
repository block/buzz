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

## Usage delivery

Set `BUZZ_STORAGE_USAGE_URL` to push each new snapshot's per-community totals
to an operator endpoint. kgoose's `POST {prefix}/v1/buzz/storage-usage/record`
receives them and bills sponsored communities in Orb. Unset, nothing is sent.
Every five minutes, one relay pod (elected by a PostgreSQL session advisory
lock) compares the stored snapshot's `completed_at` with the last one it
handled and, when it is new, POSTs it in batches of at most 500 communities:

```json
{
  "measured_at": "2026-10-30T03:04:12Z",
  "communities": [
    {
      "community_host": "acme.communities.buzz.xyz",
      "storage_bytes": 123456789,
      "storage_objects": 412
    }
  ]
}
```

- `measured_at` is the snapshot's completion time in RFC 3339, the same for
  every batch of a snapshot. Each batch is an independent request.
- Each request carries a NIP-98 `Authorization` header signed by the relay
  key, binding the URL and body, as operator-listener mention notifications
  do. kgoose authenticates the relay by its `buzz` mesh service identity
  instead, so the relay needs mesh egress to kgoose-builderbot.
- `storage_bytes` and `storage_objects` are the logical per-community values
  described above: a blob bound to several communities counts in full for
  each.
- Every snapshot community with a community row is sent, including archived
  or deleting ones that still hold bytes. An active community absent from the
  snapshot is sent with zero, so a receiver keeping the latest value never
  retains a stale one. A snapshot community with no community row has no host
  and is skipped with a warning.
- A 2xx accepts a batch. A 408, 429, 5xx, or transport error stops the run and
  re-sends the whole snapshot on the next check. Any other status abandons
  that snapshot; the next worker run is delivered normally.
- Which snapshot was handled is kept in memory, so a restart or leader change
  re-sends the latest snapshot. Receivers MUST treat a repeated community and
  `measured_at` as already recorded; kgoose bills only a community's first
  report for each UTC day.
- A snapshot replaced before it was delivered is never sent. Alert on
  `time() - buzz_storage_usage_delivered_snapshot_timestamp_seconds` alongside
  snapshot age; `buzz_storage_usage_deliveries_total{outcome}` counts
  `accepted`, `retry`, and `rejected` checks.
