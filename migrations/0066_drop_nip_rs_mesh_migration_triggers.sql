-- Drop the NIP-RS, mention-liveness and mesh-status triggers that exist only
-- in migration-managed databases. The application now owns each rule inside
-- the event transaction (crates/buzz-db/src/store/replaceable.rs and
-- crates/buzz-db/src/store/event.rs):
--
-- - it advances parameterized_event_watermarks and rejects stale NIP-RS writes;
-- - it physically deletes superseded and NIP-09-deleted NIP-RS and mesh-status
--   rows, with their mentions;
-- - it indexes mentions in the same transaction as the event, so a mention
--   can no longer race a hard delete.
--
-- schema/ never declared these objects, so desired-state databases already run
-- without them. Dropping a trigger on the partitioned parent also drops its
-- partition clones. Published migrations 0009, 0010, 0011 and 0019 stay
-- checksum-frozen.
--
-- DROP TRIGGER takes ACCESS EXCLUSIVE on events and every partition; fail the
-- deployment rather than queue relay reads and writes behind a long holder.
SET LOCAL lock_timeout = '5s';

DROP TRIGGER trg_events_nip_rs_watermark ON events;
DROP TRIGGER trg_events_guard_nip_rs_hard_delete ON events;
DROP TRIGGER trg_events_purge_soft_deleted_nip_rs ON events;
DROP TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status ON events;
DROP TRIGGER trg_event_mentions_require_live_event ON event_mentions;

DROP FUNCTION guard_nip_rs_watermark();
DROP FUNCTION guard_nip_rs_hard_delete();
DROP FUNCTION purge_soft_deleted_nip_rs();
DROP FUNCTION purge_soft_deleted_buzz_mesh_status();
DROP FUNCTION guard_event_mention_live();
