-- Audited, idempotent orphaned-channel ownership recovery.
--
-- The request, promotion, immutable audit row, and delivery outbox row are
-- committed in one transaction. The outbox permits relay-signed audit delivery
-- to be retried without repeating the promotion.

CREATE TABLE channel_owner_recovery_audit (
    community_id        UUID NOT NULL REFERENCES communities(id),
    request_event_id    BYTEA NOT NULL,
    channel_id          UUID NOT NULL,
    actor_pubkey        BYTEA NOT NULL,
    target_pubkey       BYTEA NOT NULL,
    predicate_id        TEXT NOT NULL,
    reason_code         TEXT NOT NULL,
    reason              TEXT NOT NULL,
    prior_elevated_roles JSONB NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, request_event_id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id),
    CONSTRAINT chk_owner_recovery_event_id_len CHECK (length(request_event_id) = 32),
    CONSTRAINT chk_owner_recovery_actor_len CHECK (length(actor_pubkey) = 32),
    CONSTRAINT chk_owner_recovery_target_len CHECK (length(target_pubkey) = 32),
    CONSTRAINT chk_owner_recovery_predicate_nonempty CHECK (length(btrim(predicate_id)) > 0),
    CONSTRAINT chk_owner_recovery_reason_code_nonempty CHECK (length(btrim(reason_code)) > 0),
    CONSTRAINT chk_owner_recovery_reason CHECK (
        length(btrim(reason)) > 0
        AND octet_length(btrim(reason)) <= 500
        AND reason !~ '[[:cntrl:]]'
    )
);

CREATE TABLE channel_owner_recovery_outbox (
    community_id        UUID NOT NULL REFERENCES communities(id),
    request_event_id    BYTEA NOT NULL,
    channel_id          UUID NOT NULL,
    audit_event_id      BYTEA,
    attempts            INT NOT NULL DEFAULT 0,
    delivered_at        TIMESTAMPTZ,
    last_error          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, request_event_id),
    FOREIGN KEY (community_id, request_event_id)
        REFERENCES channel_owner_recovery_audit (community_id, request_event_id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id),
    CONSTRAINT chk_owner_recovery_audit_event_id_len
        CHECK (audit_event_id IS NULL OR length(audit_event_id) = 32)
);

CREATE UNIQUE INDEX uq_channel_owner_recovery_audit_event
    ON channel_owner_recovery_outbox (community_id, audit_event_id)
    WHERE audit_event_id IS NOT NULL;

CREATE FUNCTION channel_owner_recovery_audit_immutable() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'channel owner recovery audit rows are immutable'
        USING ERRCODE = 'check_violation';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_channel_owner_recovery_audit_immutable
    BEFORE UPDATE OR DELETE ON channel_owner_recovery_audit
    FOR EACH ROW EXECUTE FUNCTION channel_owner_recovery_audit_immutable();

-- A relay binary from before this migration can still reach the legacy
-- unconditional event soft-delete primitive. Keep the exact tenant-linked
-- channel audit visible at the database boundary so mixed-version pods fail
-- closed after the schema migration lands.
CREATE FUNCTION channel_owner_recovery_event_immutable() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM channel_owner_recovery_outbox
        WHERE community_id = OLD.community_id
          AND audit_event_id = OLD.id
    ) THEN
        RAISE EXCEPTION 'channel owner recovery audit events are immutable'
            USING ERRCODE = 'check_violation';
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_events_channel_owner_recovery_soft_delete_guard
    BEFORE UPDATE OF deleted_at ON events
    FOR EACH ROW
    WHEN (OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
    EXECUTE FUNCTION channel_owner_recovery_event_immutable();

CREATE TRIGGER trg_events_channel_owner_recovery_hard_delete_guard
    BEFORE DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION channel_owner_recovery_event_immutable();
