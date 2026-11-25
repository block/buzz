-- ── Event mentions ────────────────────────────────────────────────────────────
-- Conformance: "Channel-less global events and DMs" (#p fan-out). The join to
-- events MUST carry the community tuple (e.community_id = m.community_id AND
-- e.id = m.event_id) — bare e.id = m.event_id would leak cross-community
-- mentions (Max, verified at event.rs:222).

CREATE TABLE event_mentions (
    community_id        UUID NOT NULL,
    pubkey_hex          VARCHAR(64) NOT NULL,
    event_id            BYTEA NOT NULL,
    event_created_at    TIMESTAMPTZ NOT NULL,
    channel_id          UUID,
    event_kind          INT,
    -- Relay arrival of the event (events.received_at). NULL on rows written
    -- before 0107: those are not listed by /buzz/v1/me/activity.
    received_at         TIMESTAMPTZ,
    -- Thread root when the event is a reply; NULL for a top-level message.
    root_id             BYTEA,
    -- When the tagged person read this mention on its own (mark_read). NULL:
    -- it follows the read position of its channel timeline or thread.
    read_at             TIMESTAMPTZ,
    PRIMARY KEY (community_id, pubkey_hex, event_id)
);

CREATE INDEX idx_event_mentions_pubkey_created
    ON event_mentions (community_id, pubkey_hex, event_created_at DESC);
CREATE INDEX idx_event_mentions_pubkey_kind_created
    ON event_mentions (community_id, pubkey_hex, event_kind, event_created_at DESC);
CREATE INDEX idx_event_mentions_community_event
    ON event_mentions (community_id, event_id);
CREATE INDEX idx_event_mentions_pubkey_received
    ON event_mentions (community_id, pubkey_hex, received_at DESC, event_id);
