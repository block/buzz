# Usage metrics operations

The relay runs no database aggregate queries for usage telemetry, and it emits
no per-community usage series. Its usage poller emits pod-local in-memory
gauges and republishes fleet totals from the stored storage snapshot. Fleet
counts of users, channels, members, workflows, repositories, and active users
are not relay metrics.

## In-memory gauges

Every pod exports its own `buzz_total_ws_connections`,
`buzz_total_users_online_pod`, and `buzz_total_subscriptions`. They come from
live process state, so dashboards aggregate them with `sum()` across pods.

## Storage gauges

Storage totals come from the `buzz-admin` worker snapshot and carry their own
`buzz_storage_snapshot_load_ok` and `buzz_storage_snapshot_age_seconds` health
gauges; see [storage-accounting.md](storage-accounting.md). Storage gauges are
leader-only: in a multi-pod deployment, first filter them to the pod where
`buzz_usage_poller_is_leader == 1`; do not take an unfiltered maximum across
pods.

## Leader election

Every relay process attempts the same PostgreSQL session advisory lock. Exactly
one live database session owns it. The leader reaps expired relay invites and
publishes the storage gauges. If that process or session exits, PostgreSQL
releases the lock and another relay acquires it.
