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

## Availability

Dashboards and monitors must pair values with these fixed-cardinality gauges:

- `buzz_usage_snapshot_available{family="stock"}`
- `buzz_usage_snapshot_available{family="activity"}`
- `buzz_storage_community_breakdown_available`

Database and storage gauges are leader-only. In a multi-pod deployment, first
filter them to the pod where `buzz_usage_poller_is_leader == 1`; do not take an
unfiltered maximum across pods. A demoted pod clears its snapshot availability,
but previously exported value series remain scrape-visible until the recorder
evicts them.

A value of `0` means the corresponding snapshot was not collected. It must not
be interpreted as all usage being zero. Failed stock or activity collections
retry after 60 seconds; successful collections resume their normal hourly and
daily cadences.

In totals-only storage mode, the relay does not calculate community attribution
and therefore does not emit `buzz_storage_unmapped_community_bytes`. The
breakdown-availability gauge communicates that omission.

## Rollout and rollback

1. Deploy with per-community mode unset or `off` and the telemetry replica
   freshness budget at its 30000ms default.
2. Update dashboards to use fleet gauges and gate alerts on the availability
   gauges above.
3. Remove dependencies on the retired per-community series before increasing
   community count.

`BUZZ_USAGE_METRICS_PER_COMMUNITY=all` temporarily restores the prior
per-community emission for rollback or dashboard migration. It also restores
the associated memory and monitoring-cardinality growth, so it is not the
steady-state configuration.

## Leader election

Every relay process attempts the same PostgreSQL session advisory lock. Exactly
one live database session owns it and performs collection. If that process or
session exits, PostgreSQL releases the lock and another relay acquires it. The
lock only deduplicates collectors; it does not permit writer fallback for fleet
queries. A process that loses the lock clears its cached fleet snapshots, marks
their availability as zero, and forces fresh collection if it later becomes the
leader again.
