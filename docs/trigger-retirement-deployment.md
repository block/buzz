# Trigger retirement deployment

Four migrations drop database triggers and functions whose work the relay
now does in application code. They ship together:

| Migration | Drops | Application replacement |
| --- | --- | --- |
| `0089_retire_created_at_floor.sql` | the `events_created_at_floor` constraint trigger and its function (`0021`) | the insert-time floor check, [#8325](https://github.com/block/buzz/pull/8325) |
| `0090_retire_community_write_fence.sql` | the universal community write-fence triggers and functions | app-side write admission, [#8219](https://github.com/block/buzz/pull/8219), [#8318](https://github.com/block/buzz/pull/8318) and [#8322](https://github.com/block/buzz/pull/8322) |
| `0091_drop_nip_rs_mesh_migration_triggers.sql` | the migration-only NIP-RS watermark, purge, mention-liveness and mesh-status triggers | [#8128](https://github.com/block/buzz/pull/8128) |
| `0095_retire_channel_roster_trigger.sql` | the `0032` channel-roster trigger `trg_events_guard_channel_roster_snapshot` and its function | the app-side kind:39002 roster check, [#8316](https://github.com/block/buzz/pull/8316); since [#8611](https://github.com/block/buzz/pull/8611) no relay requires the trigger at startup |

## Required stop

**A relay build at or after commit `68785206` (the merge of
[#8325](https://github.com/block/buzz/pull/8325)) and before these migrations
is a required stop.** Such a build contains every replacement above. No tagged
relay release contains #8325: `relay-v0.3.0` has the others but not #8325, so
a deployment on a tagged release must move to a stop build first.

Do not upgrade past the stop build to one with these migrations until no older
relay runs anywhere: every pod and job, not only a rollout's new pods. The
migrations cannot detect an older writer. If an operator upgrades straight
from an older build, the first new pod applies the migrations while older pods
are still serving:

- a relay without #8325 has no floor check, so it accepts backdated channel
  events that replica reads can then skip;
- a relay without #8318 fails its background jobs with
  `42883 community_write_allowed(uuid) does not exist`;
- a relay without #8128 loses the NIP-RS watermark and superseded-row purge;
- a relay without #8316 does not check kind:39002 roster snapshots against
  channel membership, so it can save stale rosters;
- a build with migration `0032` but without #8611 refuses to start once the
  roster trigger is gone, so such a pod cannot restart during the rollout or
  serve as a rollback target.

## Procedure

1. Deploy a stop build. Check the image of every relay pod and job, and
   wait until none runs an older one.
2. Stop out-of-band writers (direct SQL, `COPY`, restores, seed scripts). Hold
   the replica breaker closed from before the migrations until every replica
   has replayed them.
3. Apply the migrations off-peak. Each one sets a 5s `lock_timeout`, and the
   `DROP TRIGGER` statements take `ACCESS EXCLUSIVE` on `events` and its
   partitions, so event reads and writes can stall for up to that long. A
   lock timeout aborts that migration, which is safe to retry.
   - **Auto-migration on:** deploy the new build. Its first pod applies the
     migrations at startup.
   - **Auto-migration off:** run `buzz-admin migrate` with the target
     `DATABASE_URL`, confirm that it records 0089 through 0095, then deploy
     the new build. The new build assumes 0091 has run: against a
     database without it, NIP-RS hard deletes fail closed with
     `check_violation`. The stop build works with all four applied.
4. Once every relay runs the new build, run the one-off cleanup SQL in
   [#8211](https://github.com/block/buzz/pull/8211) once per database. While
   stop-build pods served after 0091, a replacement could leave a
   soft-deleted NIP-RS or mesh-status head that nothing purges any more. The
   cleanup is idempotent.

## Desired-state deployments

A deployment that applies `schema/` with pgschema follows the same order.
Apply the migrations first, or first drop the triggers on the partitioned
parents: `events_created_at_floor`, `community_write_fence_events` and
`trg_events_guard_channel_roster_snapshot` on `events`, and
`community_write_fence_delivery_log` on `delivery_log`.
Otherwise pgschema plans the partition copies' drops before the parent's, and
PostgreSQL refuses with `2BP01`.

## Rollback

Roll back to the stop build, not further, and only with auto-migration off
(`BUZZ_AUTO_MIGRATE=false`, or `migrate.autoMigrate=false` in the Helm chart).
With it on, the stop build refuses a database that has migrations it does
not know (`VersionMissing`) and does not start. With it off, the stop build
serves, but its startup check finds the floor trigger gone and keeps the
replica fence closed, so cursor reads go to the writer.

## Plain SQL functions (`0094`)

`0094_drop_plain_sql_functions.sql` drops two functions whose work the relay
now does in application code:

| Function | Application replacement |
| --- | --- |
| `community_deletion_lock_key(uuid)` | the Rust-built lock key, [#9396](https://github.com/block/buzz/pull/9396) |
| `profile_name_fields(text)` | inline profile-name SQL, [#9397](https://github.com/block/buzz/pull/9397) |

**A relay build at or after commit `279a31da` (the merge of
[#9397](https://github.com/block/buzz/pull/9397), which follows #9396) and
before `0094` is a required stop.** Builds from commit `a8f55e63` (the merge
of [#9363](https://github.com/block/buzz/pull/9363)) up to the merge of
[#9615](https://github.com/block/buzz/pull/9615) do not qualify: their user
content cleanup still calls `community_deletion_lock_key`. No tagged relay
release is a stop build: `relay-v0.3.0` is not, so a deployment on a tagged
release must move to a stop build first. Follow the same procedure as above. An older relay that
still serves after `0094` fails every write and deletion that takes the community deletion lock, with
`42883 community_deletion_lock_key(uuid) does not exist`, and its profile name
search fails with `42883 profile_name_fields(text) does not exist`.

`0094` takes no table locks. It needs no cleanup, and a rollback to a stop
build needs auto-migration off, as above.
