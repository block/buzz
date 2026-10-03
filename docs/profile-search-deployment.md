# Profile search deployment

Migration `0056_profile_search_text_fields.sql` changes how kind-0 profiles are
indexed for search. Before it, `events.search_tsv` tokenized the whole profile
JSON, so an inline base64 avatar contributed thousands of lexemes and one
shared agent avatar could make a hundred profiles match a name prefix none of
them carry. After it, kind 0 routes through `profile_search_tsv(content)`,
which indexes the profile's name, contact, and `about` fields only.

At relay startup the migration:

1. Refuses to run on PostgreSQL older than 16, naming the requirement
   (`profile_search_tsv` uses `pg_input_is_valid`).
2. Creates `profile_search_tsv` unconditionally.
3. Rewrites the generated column only when `events` is empty.

A populated database keeps its current expression. Replacing a generated
`STORED` column copies every partition of `events` and rebuilds the GIN index
under `ACCESS EXCLUSIVE` for as long as the copy takes, which a Kubernetes
startup probe can kill and repeat. The query-side name-first ordering in
`buzz-search` keeps profile lookups usable in the meantime, and the relay logs
this at every boot until the rewrite has run:

```text
WARN Profile search still indexes whole kind-0 JSON on this database; run the maintenance rewrite script=scripts/maintenance/profile_search_text_fields.sql procedure=docs/profile-search-deployment.md
```

## Who needs the rewrite

Any deployment that held events before starting a relay version with migration
0056. Fresh installs do not. Emptying `events` later does not help: the
migration has already run, so the old expression stays and every profile
published afterwards indexes its whole JSON again. Check without side effects,
from a shell with the relay's `DATABASE_URL`:

```sh
buzz-admin profile-search-policy
```

The command opens one read-only session and prints the live expression and
`outcome`: `ok` (exit 0), `rewrite_pending` (exit 2), or `missing` (exit 5,
the column does not exist yet). The same probe drives the startup warning, so
the two never disagree.

The equivalent SQL:

```sql
SELECT pg_get_expr(d.adbin, d.adrelid) AS expression
FROM pg_attrdef AS d
JOIN pg_attribute AS a ON a.attrelid = d.adrelid AND a.attnum = d.adnum
WHERE d.adrelid = 'events'::regclass
  AND a.attname = 'search_tsv';
```

The rewrite is pending when the expression does not mention
`profile_search_tsv(content)`.

## Sizing

The rewrite evaluates the search expression for every row of every `events`
partition, then builds the GIN index. Time is dominated by tokenizing the
indexed chat content (roughly 20 MB of content per second, single-threaded).
Transient disk is about a second copy of the heap and TOAST plus the new index
plus WAL.

```sql
SELECT kind, count(*), pg_size_pretty(sum(octet_length(content)))
FROM events GROUP BY kind ORDER BY 2 DESC;

SELECT pg_size_pretty(sum(pg_total_relation_size(inhrelid)))
FROM pg_inherits WHERE inhparent = 'events'::regclass;
```

Rehearse on a clone of the production database, time it, and schedule a
window of that length. Event reads and writes on the relay stall for the whole
window: `ACCESS EXCLUSIVE` blocks `SELECT` as well as `INSERT`, so clients see
history requests hang and publishes fail until the transaction commits.

## Running it

```sh
psql -v ON_ERROR_STOP=1 -f scripts/maintenance/profile_search_text_fields.sql
```

Requirements:

- **Table owner.** `ALTER TABLE events` requires the role that owns `events`,
  normally the relay's application role. A read-only or admin role that lacks
  ownership fails with `must be owner of table events` before changing
  anything.
- **No statement timeout.** `ADD COLUMN ... GENERATED ... STORED` runs as one
  statement for the full copy. A `statement_timeout` on the role or the psql
  wrapper (bastion tooling commonly defaults to tens of seconds) cancels it and
  rolls everything back. Disable it for the session, for example
  `SET statement_timeout = 0;` before `\i`, or use the wrapper's option for
  running without a timeout.
- **A started relay.** The script refuses to run until the relay has started
  once on 0056 and created `profile_search_tsv`.

Behaviour on failure:

- The script sets `lock_timeout = '5s'`. If a long-running writer holds a
  conflicting lock, the script fails with `canceling statement due to lock
  timeout` and nothing changes; rerun when the table is quieter. This is
  deliberate: a pending `ACCESS EXCLUSIVE` request queues every new reader and
  writer behind it, so waiting longer blocks traffic, not just the script.
- Everything runs in one transaction. A cancelled or failed run rolls back to
  the previous column and index; there is no partial state to clean up.
- Rerunning after success is a no-op: the script notices the kind-0 arm and
  returns with a `NOTICE`.

There is no cheap undo. Reverting means another full rewrite with the previous
expression, so record the output of the `pg_get_expr` query above before you
start.

## Verification

Run `buzz-admin profile-search-policy` again; `outcome` is `ok` and the
expression's kind-0 arm reads `WHEN (kind = 0) THEN profile_search_tsv(content)`.
The next relay start logs no profile-search warning.

Stored vectors shrink to the profile's text fields. Profiles with an inline
avatar previously carried thousands of lexemes; afterwards they carry a
handful:

```sql
SELECT encode(pubkey, 'hex') AS pubkey, length(search_tsv) AS lexemes
FROM events
WHERE kind = 0 AND content LIKE '%"picture":"data:%'
ORDER BY lexemes DESC
LIMIT 5;
```

## Relation to `nip_rs_search_allowlist.sql`

`scripts/maintenance/nip_rs_search_allowlist.sql` is the older rewrite that
converts a pre-0008 blocklist expression to the fresh-install allowlist. It now
produces the kind-0 arm as well, at the same cost, so a brownfield database
needs one rewrite, not two: run the allowlist script if you also want the
allowlist policy, otherwise run `profile_search_text_fields.sql`, which wraps
whatever expression the database has today. Running the profile script after
the allowlist script is a no-op.

## Changing `profile_search_tsv` later

`CREATE OR REPLACE FUNCTION` recomputes nothing already stored: every existing
row keeps the vector the old body produced, and only new or updated rows see
the change. Any later fix to the function therefore ships with its own
maintenance rewrite and this procedure, the same way 0056 does. Never `DROP`
the function: `events.search_tsv` depends on it, and a `CASCADE` drops the
column.
