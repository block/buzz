# Partition catalog monitoring

The relay audits the `events` and `delivery_log` partition catalogs at startup
and every `BUZZ_PARTITION_AUDIT_INTERVAL_SECS` (default 900 seconds). Results are
diagnostic only. They appear in `/_status.partition_catalog` and in the metrics
below, and they never change `/_readiness` or `/_liveness`. Alert on them; do not
gate traffic on them.

## Outcomes

`buzz-admin partition-audit` prints one outcome for the whole catalog.

| Outcome | Meaning | `buzz-admin` exit code |
|---|---|---|
| `ok` | Every managed table covers now and needs no operator attention. | 0 |
| `degraded` | Every table covers now, but one needs attention. | 0 |
| `unsafe` | A table cannot route a row timestamped now. | 2 |
| `error` | At least one table could not be audited. | 5 |

The relay's `buzz_partition_audit_runs_total{outcome}` is per table and is only
`ok`, `degraded`, or `error`. There is no `unsafe` outcome there: a table that
cannot route a row timestamped now is `degraded` and has
`buzz_partition_serving_safe` 0.

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

The exporter removes a gauge that has not been updated within its idle timeout.
The default is 2,700 seconds: three times the larger of the usage-metrics and
partition-audit intervals. A failed audit of a table refreshes that table's
`buzz_partition_audit_last_success_timestamp_seconds` without changing its
value, so sustained failures keep the last success exported and its age keeps
rising. A table that has never been audited successfully exports `0`. The
timestamp disappears only when audits stop running for longer than the idle
timeout; the other per-table gauges can be evicted during sustained failures.

Counters are not idle-evicted while the process runs. The relay exports every
run outcome for both managed tables, and the failure counter, at 0 when its
metrics exporter starts, so `increase()` counts the first failure and the first
run with a new outcome:

- `buzz_partition_audit_runs_total{table, outcome}` increments once per table
  per audit. The outcome is `ok`, `degraded`, or `error`, including audits run
  inside startup partition creation.
- `buzz_partition_audit_failures_total` counts failed explicit audit attempts:
  the startup fallback audit that runs when startup partition creation fails, and
  each periodic audit. A startup creation failure is not counted by itself. If
  the fallback audit then fails, that attempt counts once.
- `buzz_partition_create_attempts_total{table, outcome}` counts startup creation
  decisions.

## Alerts

Use all three rules below. Aggregate with `without (...)`, not `by (...)`, so
cluster, account, environment, namespace, and pod labels stay in the alert.

When a pod terminates, its scrape target disappears and its series go stale.
Instant selectors drop a stale series at once, but range selectors such as
`[30m]` keep reading its earlier samples until they leave the window. Each
range-based rule therefore also requires the counter to be exported now, which
it is for the whole life of a relay, so a terminated pod cannot keep or start an
alert.

```yaml
# The last success is older than two default audit intervals. This also fires
# for a table that has never audited successfully, whose timestamp is 0, and
# keeps firing through sustained failures. Scale 1800 with
# BUZZ_PARTITION_AUDIT_INTERVAL_SECS.
- alert: BuzzPartitionAuditStale
  expr: time() - buzz_partition_audit_last_success_timestamp_seconds > 1800
  for: 10m

# The relay is running but its audit loop has stopped. Use a window of about
# two audit intervals, and keep the window plus `for` below the gauge idle
# timeout (2,700 s by default), so this fires before the stopped loop's
# timestamp is evicted and the stale rule resolves.
- alert: BuzzPartitionAuditNotRunning
  expr: |
    sum without (outcome) (increase(buzz_partition_audit_runs_total[30m])) == 0
      and sum without (outcome) (buzz_partition_audit_runs_total)
  for: 5m

# Explicit audit attempts keep failing, even if a later attempt succeeds.
- alert: BuzzPartitionAuditFailing
  expr: |
    increase(buzz_partition_audit_failures_total[1h]) >= 2
      and buzz_partition_audit_failures_total
```

The counters exist from relay start, and the success timestamp exists after the
first audit attempt, whether or not it succeeded. The startup audit runs within
seconds of the zero baselines, usually before the first scrape, so
`BuzzPartitionAuditFailing` can miss a failure in that one attempt; if it does
not recover, `BuzzPartitionAuditStale` fires. To also catch a live relay that
never exports audit runs, compare its scrape target health with the run
counter, for example
`up == 1 unless on (namespace, pod) count by (namespace, pod) (buzz_partition_audit_runs_total)`,
scoped to the relay job.

`degraded` asks for operator attention; it does not mean rows cannot be
written. Because historical legacy leaves no longer count, a repaired catalog
returns to `ok` once the current month passes the repaired range. Coverage
problems show directly in `buzz_partition_serving_safe` and
`buzz_partition_uncovered_months`.

The tests in `crates/buzz-relay/src/metrics.rs` (`partition_alert_tests`) check
that the production exporter and audit emit the series these rules read: zero
counter baselines before the first audit, a `0` timestamp before any success,
the last success kept through failures past the idle timeout, and run counters
that outlive an evicted timestamp. They do not
evaluate the rules, their `for` durations, or label matching; validate those
with `promtool test rules` against your alerting configuration.
