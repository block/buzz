CREATE TABLE IF NOT EXISTS workflow_event_outbox (
    community_id   UUID NOT NULL,
    event_id       BYTEA NOT NULL,
    available_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts       INTEGER NOT NULL DEFAULT 0,
    claimed_by     TEXT,
    claimed_until  TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_workflow_event_outbox_claimable
    ON workflow_event_outbox (available_at, claimed_until);

CREATE OR REPLACE FUNCTION enqueue_workflow_event_outbox() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- Do not create a durable handoff for every ordinary channel message.
    -- Only channels with an active workflow need event-trigger recovery; this
    -- keeps the outbox proportional to workflow traffic rather than chat
    -- traffic and avoids an unbounded backlog when no workflows are enabled.
    IF NEW.channel_id IS NOT NULL AND EXISTS (
        SELECT 1
        FROM workflows w
        WHERE w.community_id = NEW.community_id
          AND w.channel_id = NEW.channel_id
          AND w.status = 'active'
          AND w.enabled
    ) THEN
        INSERT INTO workflow_event_outbox (community_id, event_id)
        VALUES (NEW.community_id, NEW.id)
        ON CONFLICT (community_id, event_id) DO NOTHING;
    END IF;
    RETURN NEW;
END
$$;

DROP TRIGGER IF EXISTS events_workflow_outbox ON events;
CREATE TRIGGER events_workflow_outbox
AFTER INSERT ON events
FOR EACH ROW EXECUTE FUNCTION enqueue_workflow_event_outbox();
