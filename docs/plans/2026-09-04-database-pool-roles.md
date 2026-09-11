# Database Pool Role Instrumentation

## Approved design

At base `e09f715c9d0ee2cb7bf8a39061e601f3a502f588`, expose one closed physical
Postgres pool-role vocabulary: `writer`, `reader`, `audit`, and `search`.
Use it only at pool construction and utilization-metrics boundaries.

Emit a fixed-cardinality, role-labelled utilization contract for all four
roles: configured state plus size, idle, active, and maximum connections.
Unconfigured optional pools report configured `0` and zero utilization so
series do not appear and disappear. Preserve the existing writer and reader
metric families for dashboard compatibility.

Keep cheap `PgPool` clones solely as statistics handles for audit and search;
service ownership and routing remain unchanged. Active utilization is derived
with saturating subtraction (`size - idle`).

This change deliberately does not add a central pool manager, change pool
capacities or timeouts, change query routing, add audit/search acquisition
telemetry, or enforce aggregate deployment connection budgets.

## Test-first implementation plan

1. Add a production-seam test proving the pool-role vocabulary contains
   exactly the four approved values and labels. Run it to RED, implement the
   closed role type, then run it to GREEN.
2. Add a production-seam test for active-connection arithmetic, including the
   defensive saturating case. Run it to RED, add the minimal statistic helper,
   then run it to GREEN.
3. Add relay metrics scrape-contract tests for exact role labels, fixed series
   cardinality, optional-reader configured state, utilization values, and the
   legacy writer/reader metric families. Run them to RED, implement one bounded
   recorder used by production, then run them to GREEN.
4. Pair writer, optional reader, optional audit, and search statistic handles
   with their roles at construction, retaining cheap pool clones for the two
   independently constructed pools. Do not alter service ownership.
5. Update the metrics documentation and only the directly relevant stale
   architecture claims about search indexing; generated-column maintenance is
   synchronous and search is query-only.
6. Run formatting and checks, complete test suites for every touched crate,
   `just test`, and `just ci`. Commit with the configured identity and DCO
   signoff, then repeat final verification while capturing the exact checked
   out HEAD in the same shell.
