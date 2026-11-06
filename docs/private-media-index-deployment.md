# Media edit target index deployment

Before enabling shadow, build and verify the partial btree on the normalized
first valid edit target. Small databases may use migration 0086 with auto-migration;
large deployments with auto-migration off prebuild concurrently before applying
0086 or desired-state schema. Signed event tags are never rewritten.
The immutable built-in JSONPath expression preserves tag order and hex casing.
No SQL function is added.

For a concurrent prebuild, use the same approach as [thread windows](thread-window-deployment.md).
Run from this checkout, outside a transaction:

```sh
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -f scripts/maintenance/prebuild-media-edit-index.sql
```

The script creates an `ON ONLY` parent, builds each immediate events partition
index `CONCURRENTLY` with PostgreSQL-generated names (matching pgschema), then
attaches it. pgschema cannot build a partitioned parent concurrently, so this
prebuild is required for large existing databases under either schema deployment
path. The final bounded migration guard checks
readiness, validity, access method, key order, predicate and exact expression.
A valid parent proves every existing partition has an attached matching index.
New monthly partitions inherit the index through ordinary `PARTITION OF` creation;
the PostgreSQL regression creates a future partition and verifies attachment.
Keep partition management paused while running the prebuild; if a new partition
appears before verification, rerun the idempotent script and verify again.

A failed concurrent child build leaves an invalid index. Run only one prebuild
at a time; ensure an earlier build has ended before retrying. A retry drops
only invalid, unattached child indexes whose definitions match the parent,
using `DROP INDEX CONCURRENTLY` outside a transaction. It then rebuilds them
concurrently with the same generated names expected by desired-state tooling.
Concurrent child creation and cleanup disable `lock_timeout` so older
transactions can finish; each statement retains its 30-minute timeout. Parent
creation and partition attachment retain the one-second lock timeout.
Valid or attached indexes are retained. Wrong definitions fail attachment or
migration verification and need operator correction before proceeding. Large
deployments must retain the concurrent build procedure.

Migration 0086 builds a missing index inline on empty or populated databases,
with a one-second lock timeout and five-second statement timeout. Contention or
a build exceeding those budgets aborts the migration without recording 0086;
retry after the contention clears, or disable auto-migration and prebuild
concurrently for a large deployment. A prebuilt index skips CREATE entirely,
then passes the same exact catalog validation without an index-build lock.
With auto-migration disabled, operators must apply and verify 0086 before deploying
this binary, even in off mode. Bypassing that ordering and enabling enforce can
cause lookup timeouts and authorization-unavailable responses; restore off and
complete prebuild/verification before shadow. Do not use that bypass as a soak.

This is an additive expand/deploy change: existing relay reads and writes ignore
the new index, which adds only normal index maintenance. No row backfill, trigger,
function or foreign key is added. New and old binaries may serve against the
expanded schema. Keep auto-migration disabled on old binaries/restarts/rollback
because sqlx rejects unknown migration versions. Do not run community deletion
between 0086's table expansion and the new binary's deletion-catalog deployment.

Migration 0086 builds the index and expands the media ownership tables in one
transaction, with the fallible index build first. An index timeout leaves the previously applied migration
version unchanged, so an older auto-migrating binary can still restart. After
successful expansion, the usual rollback requirement above still applies.

The earlier PR snapshot split this unmerged expansion across migrations 0084
and 0085. They were consolidated during review, then renumbered to 0086 after the base added
its unrelated 0084 default-channel recovery and 0085 community-policy migrations.
Both base migrations are unchanged.
A disposable database that already applied an earlier PR snapshot must be
recreated before testing this revision; do not rewrite its sqlx migration rows
or checksums. Keep any ongoing old-snapshot trial pinned to its matching binary.
Merged migration history remains unchanged.
