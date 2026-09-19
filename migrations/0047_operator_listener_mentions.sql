-- Deployment-global operator-listener mention delivery.
-- Registrations span communities; community_id on the queues/outbox is event
-- provenance, not a tenant boundary.

CREATE TABLE operator_listener_pubkeys (
    listener_pubkey BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey   BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (listener_pubkey, target_pubkey)
);
CREATE INDEX operator_listener_pubkeys_target
    ON operator_listener_pubkeys (target_pubkey, listener_pubkey);

CREATE TABLE operator_listener_match_queue (
    community_id    UUID NOT NULL REFERENCES communities(id),
    event_id        BYTEA NOT NULL CHECK (length(event_id) = 32),
    state           TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'matching')),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until     TIMESTAMPTZ,
    claim_id        UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, event_id)
);
CREATE INDEX operator_listener_match_queue_due
    ON operator_listener_match_queue (next_attempt_at, created_at)
    WHERE state = 'pending';
CREATE INDEX operator_listener_match_queue_recovery
    ON operator_listener_match_queue (lease_until)
    WHERE state = 'matching';

CREATE TABLE operator_listener_outbox (
    id              UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    listener_pubkey BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey   BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    community_id    UUID NOT NULL REFERENCES communities(id),
    event_id        BYTEA NOT NULL CHECK (length(event_id) = 32),
    event_kind      INTEGER NOT NULL,
    event_created_at TIMESTAMPTZ NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'sending')),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until     TIMESTAMPTZ,
    claim_id        UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (listener_pubkey, target_pubkey, community_id, event_id)
);
CREATE INDEX operator_listener_outbox_due
    ON operator_listener_outbox (next_attempt_at, created_at)
    WHERE state = 'pending';
CREATE INDEX operator_listener_outbox_recovery
    ON operator_listener_outbox (lease_until)
    WHERE state = 'sending';

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('operator_listener_pubkeys', 'deployment-global target registrations for operator listeners'),
    ('operator_listener_match_queue', 'deployment-global mention matching queue; community_id is event provenance'),
    ('operator_listener_outbox', 'deployment-global webhook delivery queue; community_id is event provenance');

-- These queue tables carry community provenance but are deployment-global
-- sidecars. They must remain writable while a community is fenced so workers
-- can drain or expire their rows; they are not part of the community deletion
-- catalog.
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations', 'operator_listener_match_queue',
        'operator_listener_outbox'
    ]::TEXT[])
$$;

-- Mention indexing already happens transactionally after an accepted event.
-- Enqueue from that index so the matcher never races the separate mention
-- indexing transaction used by the existing event write path.
CREATE OR REPLACE FUNCTION enqueue_operator_listener_match_job() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM operator_listener_pubkeys LIMIT 1) THEN
        INSERT INTO operator_listener_match_queue (community_id, event_id)
        VALUES (NEW.community_id, NEW.event_id)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER event_mentions_enqueue_operator_listener_match
AFTER INSERT ON event_mentions
FOR EACH ROW WHEN (NEW.event_kind IN (9, 40002, 45001, 45003))
EXECUTE FUNCTION enqueue_operator_listener_match_job();
