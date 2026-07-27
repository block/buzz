-- T1b push gate: skip push_match_queue enqueue entirely for communities with
-- no active, endpoint-enabled, unexpired push lease. In lease-less communities
-- (most of them) every durable message currently pays the full matcher cost
-- (enqueue + claim + lease scan + delete) to conclude "notify no one".
--
-- Correctness protocol (write-amp plan rev 3, [R2/R3]):
--   * The gate lives HERE, in the events trigger, so every durable producer is
--     covered — including internal paths that bypass live dispatch.
--   * Lost-wake race: a naive EXISTS check could read "no lease" while a lease
--     activation commits concurrently, silently dropping that user's wake with
--     no retry. Closed with a per-community advisory lock held to transaction
--     end: event inserts take the lock SHARED (concurrent with each other),
--     lease transitions that can make eligibility true take it EXCLUSIVE
--     (crates/buzz-db/src/push.rs: accept_lease_event and replace_lease).
--     The conflict forces a total order: either the event's check sees the
--     committed lease, or the activation strictly follows the event's commit —
--     in which case no lease existed when the event was accepted and no wake
--     was owed. The lease-activation backfill is product recovery coverage
--     only and is not part of this proof.
--   * Lock key domain 'buzz_push_gate:' is distinct from the audit lock
--     ('buzz_audit:') and both lease-address lock families.
CREATE OR REPLACE FUNCTION enqueue_push_match_job() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- Keep this allowlist identical to the relay's validated NIP-PL descriptor.
    IF NEW.kind IN (7, 9, 1059, 40007, 46010) THEN
        PERFORM pg_advisory_xact_lock_shared(
            hashtextextended('buzz_push_gate:' || NEW.community_id::text, 0));
        IF EXISTS (
            SELECT 1 FROM push_leases
            WHERE community_id = NEW.community_id
              AND active
              AND endpoint_enabled
              AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        ) THEN
            INSERT INTO push_match_queue (community_id, event_id)
            VALUES (NEW.community_id, NEW.id)
            ON CONFLICT DO NOTHING;
        END IF;
    END IF;
    RETURN NEW;
END
$$;
