# Workflow wake FTS rollout (PostgreSQL 17)

Kind 44620 is durable workflow delivery, not searchable chat. Its `search_tsv`
must be SQL NULL even for empty or malformed private content. An empty vector
is not equivalent: it matches NOT-only queries. Neither query-layer filtering
nor the canonical empty wake payload replaces this storage contract.

## Decision

Migration 0044 retains **stored generated** search vectors. It inspects the
parent and every existing partition under a tree-wide lock. Exact PostgreSQL
catalog expression comparisons recognize the fresh positive allowlist, its
0014/0033-wrapped form, and the desired-schema policy including 44620. These
installations perform **no heap or index rewrite**. This is a conservative
recognizer, not an arbitrary SQL equivalence checker.

Other uniform generated policies are wrapped with `CASE WHEN kind = 44620 THEN
NULL::tsvector ELSE existing_expression END` using PostgreSQL 17's `SET
EXPRESSION`. This preserves other kinds' search policy and the column,
dependent view and index definitions/options; it **does rewrite heaps and
indexes**. Index OIDs/physical files can change. This is a maintenance operation,
not an online migration. Unknown custom expressions are not assumed safe.
Divergent parent/partition expressions and non-generated columns fail before
mutation: operators must reconcile that drift before upgrading, not silently
lose a leaf's custom policy.

A tested alternative, `DROP EXPRESSION` plus an ALWAYS write trigger, can retain
existing heap/index files and repair only historical wake vectors. We have not
chosen it: it adds permanent bootstrap, restoration and replication obligations,
and row-level repair fails for deletion-fenced historical wakes. There is no
assumption that pre-existing kind-44620 rows cannot exist. Generated-expression
recomputation repairs their unsigned projection without modifying signed event
fields, executing row UPDATE hooks, or granting a deletion-fence bypass. The
fence remains effective for ordinary writes.

## Before upgrade

- Inventory the generated expressions for `events.search_tsv` across
  `pg_partition_tree('events')`; record heap/index/TOAST sizes, free disk,
  replica lag and WAL retention. Do not infer the policy from an empty-content
  probe or a substring match. The same PostgreSQL-normalized whole-expression
  comparison used in the migration is authoritative for its skip path.
- Confirm PostgreSQL 17 and the repository's normal schema/destruction lock
  discipline. Migration startup must not race tenant destruction.
- For an unrecognized policy, size and schedule a maintenance window. Budget
  replacement heap/index storage plus WAL/replica headroom. No production
  duration or throughput estimate is claimed by the small disposable tests.
- Lock acquisition is limited to five seconds; a busy table makes the migration
  fail transactionally for a later controlled retry. Once acquired, an unsafe
  policy holds ACCESS EXCLUSIVE for its rewrite. Existing operator
  `statement_timeout` still applies. Even the safe skip path briefly blocks
  readers/writers while inspecting the tree; it is not lock-free.

Fresh desired-state bootstrap already includes 44620 in its generated policy;
no new reconciliation trigger or seed DML is needed. Future partitions inherit
that policy. Ordered migration bootstrap keeps the fresh positive allowlist.
These paths intentionally preserve their pre-existing search differences for
other kinds. Direct vector assignments remain rejected by PostgreSQL.

## Evidence and limits

`postgres_workflow_wake_fts` exercises safe-policy heap/index relfilenode
preservation, unsafe-policy correction with custom indexes and a dependent view,
raw NULL/NOT-only semantics, future partitions and direct leaf writes,
kind/content changes, rejected direct vector assignment, and divergent-policy
rollback. `postgres_fts_integration` also covers every persistent p-gated kind
under both fresh and legacy policies, with empty and nonempty payloads.

A disposable current desired-schema test additionally established correction of
an existing wake in a genuinely deletion-fenced community, with executor bypass
settings cleared before migration and ordinary UPDATE still rejected afterward.
These are correctness checks, not a production benchmark or deployment approval.
No live database modification is part of this PR's validation.
