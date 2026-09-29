# Partition catalog monitoring

The relay audits the `events` and `delivery_log` partition catalogs at startup
and every `BUZZ_PARTITION_AUDIT_INTERVAL_SECS` (default 900 seconds). Results are
diagnostic only. They appear in `/_status.partition_catalog` and in the metrics
below, and they never change `/_readiness` or `/_liveness`. Alert on them; do not
gate traffic on them.

## Outcomes

`buzz-admin partition-audit` prints one outcome. The relay uses the same
per-table verdict for `buzz_partition_audit_runs_total{outcome}`.

| Outcome | Meaning | `buzz-admin` exit code |
|---|---|---|
| `ok` | Every managed table covers now and needs no operator attention. | 0 |
| `degraded` | Every table covers now, but one needs attention. | 0 |
| `unsafe` | A table cannot route a row timestamped now. | 2 |
| `error` | At least one table could not be audited. | 5 |

A table is `degraded` when it has a partition-key mismatch, a `DEFAULT` or
anomalous child, trigger drift, an occupied catch-all or `DEFAULT` leaf, or a
current or lookahead month not covered by a bounded leaf. A bounded legacy leaf
with a non-canonical name degrades the table while it covers the current month or
later. Once its upper bound is at or before the start of the current month, it is
historical. It remains in the per-child report as `legacy_leaf` but no longer
degrades the table.

## Metrics

Per-table gauges are set only by a successful audit of that table:
`buzz_partition_audit_last_success_timestamp_seconds`,
`buzz_partition_serving_safe`, `buzz_partition_uncovered_months`,
`buzz_partition_catch_all_covered_months`, `buzz_partition_default_covered_months`,
`buzz_partition_anomalous_children`, `buzz_partition_catch_all_nonempty`,
`buzz_partition_default_nonempty`, `buzz_partition_trigger_parity_missing`, and
`buzz_partition_trigger_parity_extra`. A failed audit leaves them unchanged, so a
failure never makes the last success look fresh.

The exporter removes a gauge that has not been set within its idle timeout. The
default is 2,700 seconds: three times the larger of the usage-metrics and
partition-audit intervals. Sustained audit failures therefore remove the last
success timestamp, and a table that has never been audited successfully has no
timestamp at all. An alert based only on timestamp age has no input in either
case.

Counters are not idle-evicted while the process runs:

- `buzz_partition_audit_runs_total{table, outcome}` increments once per table
  per audit. The outcome is `ok`, `degraded`, or `error`, including audits run
  inside startup partition creation. After the first audit, every live relay
  exports this counter for both managed tables.
- `buzz_partition_audit_failures_total` counts failed explicit audit attempts:
  the startup fallback audit that runs when startup partition creation fails, and
  each periodic audit. A startup creation failure is not counted by itself. If
  the fallback audit then fails, that attempt counts once.
- `buzz_partition_create_attempts_total{table, outcome}` counts startup creation
  decisions.

## Alerts

Use all three rules below. The run counter defines the expected live instances
and managed tables. When a pod terminates, its scrape target disappears and its
series go stale. Normal pod turnover therefore cannot leave a persistent
missing-series alert. Aggregate with `without (...)`, not `by (...)`, so
cluster, account, environment, namespace, and pod labels stay in the alert.

```yaml
# A live relay audits this table but exports no success timestamp. This fires
# when a table has never audited successfully, and after sustained failures
# have evicted a previous success timestamp.
- alert: BuzzPartitionAuditSuccessMissing
  expr: |
    max without (outcome) (buzz_partition_audit_runs_total)
      unless buzz_partition_audit_last_success_timestamp_seconds
  for: 15m

# The last success is older than two default audit intervals. Scale 1800 with
# BUZZ_PARTITION_AUDIT_INTERVAL_SECS. Keep threshold plus `for` below the gauge
# idle timeout (2,700 s by default) so this fires before the absence rule
# takes over.
- alert: BuzzPartitionAuditStale
  expr: time() - buzz_partition_audit_last_success_timestamp_seconds > 1800
  for: 10m

# Explicit audit attempts keep failing, even if a later attempt succeeds.
- alert: BuzzPartitionAuditFailing
  expr: increase(buzz_partition_audit_failures_total[1h]) >= 2
```

To also catch a live relay that never exports audit runs, compare its scrape
target health with the run counter, for example
`up == 1 unless on (namespace, pod) count by (namespace, pod) (buzz_partition_audit_runs_total)`,
scoped to the relay job.

Backends without set subtraction can express the absence rule as "every audit
of this table on this pod failed in the window." Divide the per-pod, per-table
count of `outcome="error"` runs by all runs over a window of at least four audit
intervals. Alert when the ratio is 1 and there were attempts. Both queries read
counters, so this works after the timestamp gauge is evicted.

`degraded` asks for operator attention; it does not mean rows cannot be
written. Because historical legacy leaves no longer count, a repaired catalog
returns to `ok` once the current month passes the repaired range. Coverage
problems show directly in `buzz_partition_serving_safe` and
`buzz_partition_uncovered_months`.

The tests in `crates/buzz-relay/src/metrics.rs` (`partition_alert_tests`) check
that the production exporter and audit emit the series these rules need, in
both the never-successful and evicted-after-success cases.
