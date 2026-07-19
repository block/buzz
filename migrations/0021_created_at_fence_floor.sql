-- Replica-fence floor: make the ingest-time created_at envelope a commit-time
-- storage invariant for channel-window rows.
--
-- Why: cursor (keyset) pages may be served by a read replica behind a
-- "fence" timestamp. The fence proof requires that once a replica has
-- replayed past a sampled writer LSN, no transaction can later commit an
-- events row whose created_at is older than (commit time - floor). The
-- ingest handler checks |created_at - now| <= 900s at acceptance time, but
-- acceptance and commit are separated by unbounded async work, and several
-- writers (workflow sink, side effects, replace_*) bypass ingest entirely.
--
-- Mechanism: a DEFERRABLE INITIALLY DEFERRED constraint trigger runs inside
-- COMMIT processing, re-evaluating clock_timestamp() (NOT now(), which is
-- frozen at transaction start) so the bound is measured at commit, not at
-- INSERT. A transaction that holds an old-created_at insert open past the
-- floor budget is aborted at COMMIT and can never introduce a below-fence
-- row. Verified on PostgreSQL 16 against a partitioned table.
--
-- Scope: rows with channel_id IS NOT NULL — exactly the rows that channel
-- windows and thread pagination serve. channel_id-NULL rows (push leases,
-- profile/discovery snapshots) legitimately carry client-signed historical
-- timestamps and never appear in keyset-paged windows.
--
-- Enforcement is opt-in per session via the buzz.created_at_floor GUC
-- (seconds). The relay's writer pool sets it on every connection
-- (after_connect); when the GUC is unset or blank the guard is a no-op so
-- pg_restore/backfills and test fixtures that legitimately write historical
-- rows keep working. There is deliberately NO in-band bypass for
-- channel-bearing rows (the only structural exemption is channel_id IS
-- NULL, which never appears in keyset-paged windows): any operational
-- backfill of channel rows must run on a connection without the GUC — i.e.
-- outside the relay's writer pool — and the operator must hold the replica
-- breaker closed from before the backfill transaction begins until its WAL
-- is replayed on the replica (see Db::read routing docs). Disabling
-- triggers via session_replication_role = replica (pg_restore) is likewise
-- a breaker-closed operation.
--
-- Partition coverage: a constraint trigger created on the partitioned
-- parent is cloned onto every existing partition and onto partitions
-- created later (`CREATE TABLE .. PARTITION OF`), so partition rotation
-- keeps the guard. Row-level triggers also fire for COPY. Coverage across
-- the partition topology is asserted by a buzz-db test.

CREATE FUNCTION events_created_at_floor_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    floor_secs numeric := nullif(current_setting('buzz.created_at_floor', true), '')::numeric;
BEGIN
    IF floor_secs IS NOT NULL
       AND floor_secs > 0
       AND NEW.channel_id IS NOT NULL
       AND NEW.created_at < clock_timestamp() - make_interval(secs => floor_secs)
    THEN
        RAISE EXCEPTION
            'events.created_at % is more than % s before commit time %; below the replica-fence floor',
            NEW.created_at, floor_secs, clock_timestamp()
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

-- INSERT OR UPDATE OF: an UPDATE can move a previously exempt row into the
-- guarded set (channel_id NULL -> NOT NULL) or move a channel row's
-- created_at below the fence, so both mutation paths re-run the guard on the
-- NEW row. Partition-key note: a created_at rewrite that crosses partition
-- bounds is executed as DELETE + INSERT, which fires the cloned AFTER INSERT
-- guard on the destination partition; an in-partition rewrite fires the
-- UPDATE OF arm. Either way the NEW row is checked.
CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();
