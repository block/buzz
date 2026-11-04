# Usage metrics operations

The relay runs no database aggregate queries for usage telemetry. Its usage
poller emits pod-local in-memory gauges and republishes the stored storage
snapshot. Fleet counts of users, channels, members, workflows, repositories, and
active users are not relay metrics.

The poller is bounded-cardinality by default:

```text
BUZZ_USAGE_METRICS_PER_COMMUNITY=off
```

## In-memory gauges

Every pod exports its own `buzz_total_ws_connections`,
`buzz_total_users_online_pod`, and `buzz_total_subscriptions`. They come from
live process state, so dashboards aggregate them with `sum()` across pods.

## Storage gauges

Storage totals come from the `buzz-admin` worker snapshot and carry their own
`buzz_storage_snapshot_load_ok` and `buzz_storage_snapshot_age_seconds` health
gauges. Storage gauges are leader-only: in a multi-pod deployment, first filter
them to the pod where `buzz_usage_poller_is_leader == 1`; do not take an
unfiltered maximum across pods. In fleet-only mode the relay does not attribute
storage to communities, so it emits neither `buzz_community_storage_*` series
nor `buzz_storage_unmapped_community_bytes`.

## Per-community series

`BUZZ_USAGE_METRICS_PER_COMMUNITY=all` adds per-community in-memory series
(`buzz_community_ws_connections`, `buzz_community_users_online_pod`,
`buzz_community_subscriptions`) and per-community storage attribution. Each
tick reads the community id → host map to label them. This multiplies
monitoring cardinality by the community count, so it is not the steady-state
configuration.

## Leader election

Every relay process attempts the same PostgreSQL session advisory lock. Exactly
one live database session owns it. The leader reaps expired relay invites and
publishes the storage gauges. If that process or session exits, PostgreSQL
releases the lock and another relay acquires it.
