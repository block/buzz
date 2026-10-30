# Usage metrics operations

The relay emits fleet-wide usage and storage gauges without retaining one
in-memory series per community. This is the default, bounded-cardinality mode:

```text
BUZZ_USAGE_METRICS_PER_COMMUNITY=off
BUZZ_USAGE_METRICS_REPLICA_MAX_AGE_MS=30000
```

Database-backed fleet gauges are replica-only. The replica freshness budget is
independent of `BUZZ_REPLICA_READ_MAX_AGE_MS`, which controls serving reads. A
missing, stale, or unavailable reader skips the telemetry query; the relay never
falls back to the writer for these aggregates. Set the telemetry budget to `0`
to disable the database-backed families explicitly.

A deployment with no `READ_DATABASE_URL`, or whose reader fails verification,
therefore emits none of the database-backed fleet totals (`buzz_total_users`,
`buzz_total_channels`, `buzz_total_active_users`, and the rest of the stock
family) and reports both families unavailable. That includes local compose and
single-node self-hosts. Configure a verified reader where those totals are
needed.

## Availability

Dashboards and monitors must pair values with these fixed-cardinality gauges:

- `buzz_usage_snapshot_available{family="stock"}`
- `buzz_usage_snapshot_available{family="activity"}`

Database and storage gauges are leader-only. In a multi-pod deployment, first
filter them to the pod where `buzz_usage_poller_is_leader == 1`; do not take an
unfiltered maximum across pods. A demoted pod clears its snapshot availability,
but previously exported value series remain scrape-visible until the recorder
evicts them.

Availability reports whether the latest due collection of that family
succeeded. A value of `0` means it was skipped or failed; it must not be
interpreted as all usage being zero. The last successful values stay
scrape-visible while availability is `0`, and
`buzz_usage_snapshot_age_seconds{family=...}` reports how old they are. Failed
stock or activity collections retry after 60 seconds; a successful retry
restores availability and resumes the normal hourly and daily cadences.

Each skipped or failed collection increments
`buzz_usage_query_skipped_total{family,reason}`: `reader_unavailable` when no
fresh proved reader was available, `query_error` when the reader query failed,
and `timeout` when the whole collection (reader proof included) exceeded its
relay-side deadline of 10 seconds for stock or 20 seconds for activity. The
deadline bounds a reader that stops answering, which the server-side statement
timeout cannot, so a hung reader cannot stall the leader's poller. Telemetry
reader connections are closed when the collection ends rather than returned to
the reader pool, and an abandoned one is closed within 5 seconds, so an
abandoned telemetry read cannot hold a slot that serving reads need.

Storage totals come from the `buzz-admin` worker snapshot and carry their own
`buzz_storage_snapshot_load_ok` and `buzz_storage_snapshot_age_seconds` health
gauges. In fleet-only mode the relay does not attribute storage to communities,
so it emits neither `buzz_community_storage_*` series nor
`buzz_storage_unmapped_community_bytes`.

Every fleet telemetry query attempt that completes is recorded in
`buzz_db_route_decision{path=~"usage_fleet_.*"}`: `replica/fresh` when it ran on
a proved reader, `skipped/<reason>` when no fresh proved reader was available,
and `skipped/replica_error` when the reader query itself failed. A collection
the relay abandons at its deadline records no route decision; it appears only
as `buzz_usage_query_skipped_total{reason="timeout"}`. None of these paths fall
back to the writer.

## Rollout and rollback

1. Deploy with per-community mode unset or `off` and the telemetry replica
   freshness budget at its 30000ms default.
2. Update dashboards to use fleet gauges and gate alerts on the availability
   gauges above.
3. Remove dependencies on the retired per-community series before increasing
   community count.

`buzz_total_messages` and `buzz_total_active_channels` are emitted only by the
`all` collection. In the default fleet-only mode they stop updating, so move
dashboards off them before rollout.

`BUZZ_USAGE_METRICS_PER_COMMUNITY=all` temporarily restores the prior
per-community emission for rollback or dashboard migration. It also restores
the associated memory and monitoring-cardinality growth, so it is not the
steady-state configuration. In this mode every leader tick collects all
families, so a successful collection reports both availability gauges as `1`
and a failed one demotes the leader and reports `0`; dashboards gated on
availability or `buzz_usage_snapshot_age_seconds` keep working through a
rollback. It emits the exact
`buzz_communities_total` instead of `buzz_communities_estimated`.

## Leader election

Every relay process attempts the same PostgreSQL session advisory lock. Exactly
one live database session owns it and performs collection. If that process or
session exits, PostgreSQL releases the lock and another relay acquires it. The
lock only deduplicates collectors; it does not permit writer fallback for fleet
queries. A process that loses the lock clears its cached fleet snapshots, marks
their availability as zero, and forces fresh collection if it later becomes the
leader again.
