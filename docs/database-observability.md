# Database pressure observability

Buzz exposes bounded-cardinality metrics that separate four application-visible components of database pressure. These are measurements only: they do not set PostgreSQL timeouts, retry failed work, resize pools, or change client-visible conflict behavior.

## Metrics and fixed labels

All duration values are recorded in seconds. All counter values are monotonically increasing counts.

| Metric | Unit | Fixed labels | Measurement |
|---|---|---|---|
| `buzz_db_operation_duration_seconds` | seconds | `operation`, `outcome=success|error` | Complete body of a `#[datastore_span]` logical operation. |
| `buzz_db_pool_acquire_wait_seconds` | seconds | `pool_role=writer|reader`, `outcome=success|error|timeout` | Explicit `PgPool::acquire()` wait. |
| `buzz_db_pool_acquisitions_total` | count | `pool_role=writer|reader`, `outcome=success|error|timeout` | Explicit checkout attempts through the same helper. |
| `buzz_db_advisory_lock_wait_seconds` | seconds | `lock_type`, `outcome=success|error|timeout` | Await time for one blocking advisory-lock statement. |
| `buzz_db_advisory_lock_acquisitions_total` | count | `lock_type`, `outcome=success|error|timeout` | Blocking advisory-lock attempts through the same helper. |
| `buzz_db_transaction_duration_seconds` | seconds | `operation`, `outcome=success|error` | Selected transaction lifetimes wholly owned by `buzz-db`. |

`lock_type` is one of `replacement`, `membership`, `push_gate`, `deletion`, or `migration_schema_safety`.

Transaction `operation` is one of `replace_parameterized_event`, `replace_addressable_event`, `publish_nip43_membership_locked`, `accept_push_lease_event`, `begin_community_deletion_quiescing`, or `fence_community_deletion`.

Logical-operation names come from the compile-time string literal required by `#[datastore_span(name = "...", system = "postgresql")]`. The current vocabulary is therefore the finite set of names in those reviewed annotations; adding or changing a series requires a source change and compilation. Names cannot be supplied from request data at runtime. No metric uses community IDs, event IDs, event kinds, coordinates, d-tags, SQL/query text, or query identifiers.

`timeout` is emitted only when SQLx reports `PoolTimedOut` or a blocking advisory-lock statement returns PostgreSQL SQLSTATE `55P03`. Other failures are `error`.

## Exact boundaries

Logical operation duration starts immediately before the annotated function body and stops after it resolves. It can include an implicit SQLx checkout, advisory-lock wait, nested datastore calls, and application-side processing. It is not pure PostgreSQL statement execution time, so nested annotated calls may intentionally produce overlapping samples. A future cancelled before the body resolves does not reach the completion hook and therefore emits neither this duration nor a slow warning.

Pool acquisition duration covers explicit checkouts routed through the instrumentation helper. Writer coverage includes caller-owned `Db::begin_transaction`, the usage-metrics leader checkout, migration lock checkout, and the selected internally owned transactions listed above. Reader coverage includes the boot reachability checkout and the proved-reader checkout used by replica routing. Passing `&PgPool` directly to SQLx performs an implicit checkout that SQLx does not expose separately at the current API seam; those waits remain folded into logical operation duration. Initial minimum-pool connection establishment is also outside this metric. Reader timeout fallback and writer routing are unchanged.

Advisory-lock duration wraps only the existing lock statement. The SQL, key, blocking behavior, lock ordering, and transaction/session scope are unchanged. Coverage includes application-side replacement, membership/ownership, push lease/gate, community deletion, and migration/schema-safety locks. Advisory locks taken inside PostgreSQL triggers, stored functions, or migration SQL cannot be timed independently by the application. The channel-TTL transition lock, usage-leadership try-lock, and the separate audit service session lock remain outside the fixed families in this slice.

Transaction duration starts after `BEGIN` succeeds and stops after explicit commit/rollback completes or the Rust scope exits on error. It excludes pool wait and `BEGIN`, which are represented by the acquisition and logical-operation metrics. On an early return that drops a transaction, the timer stops at scope exit and does not include SQLx's asynchronous rollback cleanup. Caller-owned transactions returned by `Db::begin_transaction` are deliberately not wrapped in a new public transaction type, so their complete lifetime is not measured.

## Slow-operation warnings

An annotated logical operation that takes at least 500 ms is eligible for a warning. Sampling is deterministic per call site: the first slow occurrence is logged, followed by one of every 100 slow occurrences. The warning is emitted as a root event so logging formatters cannot attach fields from the surrounding datastore span. It contains only the static `operation`, fixed `outcome`, and integer `elapsed_ms`; it never formats function arguments, returned errors, SQL, or event content.

These measurements provide the evidence layer that was unavailable when PR #6229 selected timeout policy. They neither duplicate that PR's session settings nor address its separate audit durability/retry review finding. Use measured distributions by workload before changing timeout, retry, pool-budget, or routing policy in later phases.
