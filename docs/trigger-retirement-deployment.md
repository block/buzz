# Trigger retirement deployment

Three migrations drop database triggers and functions whose work the relay
now does in application code. They ship in one release:

| Migration | Drops | Application replacement |
| --- | --- | --- |
| `0089_retire_created_at_floor.sql` | the `events_created_at_floor` constraint trigger and its function (`0021`) | the insert-time floor check, [#8325](https://github.com/block/buzz/pull/8325) |
| `0090_retire_community_write_fence.sql` | the universal community write-fence triggers and functions | app-side write admission, [#8219](https://github.com/block/buzz/pull/8219), [#8318](https://github.com/block/buzz/pull/8318) and [#8322](https://github.com/block/buzz/pull/8322) |
| `0091_drop_nip_rs_mesh_migration_triggers.sql` | the migration-only NIP-RS watermark, purge, mention-liveness and mesh-status triggers | [#8128](https://github.com/block/buzz/pull/8128) |

## Required stop

**The first relay release that contains
[#8325](https://github.com/block/buzz/pull/8325) is a required stop.** It also
contains every other replacement above. `relay-v0.3.0` has the others but not
#8325, so it is not enough.

Do not upgrade past the stop to the release with these migrations until no
older relay runs anywhere: every pod and job, not only a rollout's new pods.
The migrations cannot detect an older writer. If an operator upgrades straight
from an older release, the first new pod applies the migrations while older
pods are still serving:

- a relay without #8325 has no floor check, so it accepts backdated channel
  events that replica reads can then skip;
- a relay without #8318 fails its background jobs with
  `42883 community_write_allowed(uuid) does not exist`;
- a relay without #8128 loses the NIP-RS watermark and superseded-row purge.

## Procedure

1. Deploy the stop release. Check the image of every relay pod and job, and
   wait until none runs an older one.
2. Stop out-of-band writers (direct SQL, `COPY`, restores, seed scripts). Hold
   the replica breaker closed from before the migrations until every replica
   has replayed them.
3. Apply the migrations off-peak. Each one sets a 5s `lock_timeout`, and the
   `DROP TRIGGER` statements take `ACCESS EXCLUSIVE` on `events` and its
   partitions, so event reads and writes can stall for up to that long. A
   lock timeout aborts that migration, which is safe to retry.
   - **Auto-migration on:** deploy the new release. Its first pod applies the
     migrations at startup.
   - **Auto-migration off:** run `buzz-admin migrate` with the target
     `DATABASE_URL`, confirm that it records 0089, 0090 and 0091, then deploy
     the new release. The new release assumes 0091 has run: against a
     database without it, NIP-RS hard deletes fail closed with
     `check_violation`. The stop release works with all three applied.
4. Once every relay runs the new release, run the one-off cleanup SQL in
   [#8211](https://github.com/block/buzz/pull/8211) once per database. While
   stop-release pods served after 0091, a replacement could leave a
   soft-deleted NIP-RS or mesh-status head that nothing purges any more. The
   cleanup is idempotent.

## Desired-state deployments

A deployment that applies `schema/` with pgschema follows the same order.
Apply the migrations first, or first drop the triggers on the partitioned
parents: `events_created_at_floor` and `community_write_fence_events` on
`events`, and `community_write_fence_delivery_log` on `delivery_log`.
Otherwise pgschema plans the partition copies' drops before the parent's, and
PostgreSQL refuses with `2BP01`.

## Rollback

Roll back to the stop release, not further, and only with auto-migration off
(`BUZZ_AUTO_MIGRATE=false`, or `migrate.autoMigrate=false` in the Helm chart).
With it on, the stop release refuses a database that has migrations it does
not know (`VersionMissing`) and does not start. With it off, the stop release
serves, but its startup check finds the floor trigger gone and keeps the
replica fence closed, so cursor reads go to the writer.
