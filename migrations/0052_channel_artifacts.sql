-- NIP-AR current heads and acceptance ledger are independent of event retention.
CREATE TABLE artifact_heads (
    community_id UUID NOT NULL REFERENCES communities(id),
    artifact_id UUID NOT NULL,
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    channel_id UUID NOT NULL,
    artifact_type TEXT NOT NULL,
    root BYTEA,
    deleted BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (community_id, artifact_id)
);
CREATE INDEX artifact_heads_event ON artifact_heads (community_id, event_id);
-- Every accepted revision ID, so replays stay idempotent after redaction or
-- retention.
CREATE TABLE artifact_revisions (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    artifact_id UUID NOT NULL,
    PRIMARY KEY (community_id, event_id)
);

SELECT attach_community_write_fence('artifact_heads');
SELECT attach_community_write_fence('artifact_revisions');

-- Row retention may expire earlier payloads, never the live CAS head. Partition
-- retirement must also preserve head payloads; this relay does not drop partitions.
-- A redacted (soft-deleted) head payload may still be purged.
CREATE FUNCTION retain_current_artifact() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.kind = 45010 AND OLD.deleted_at IS NULL AND EXISTS (
        SELECT 1 FROM artifact_heads h
        WHERE h.community_id=OLD.community_id AND h.event_id=OLD.id
    ) THEN
        RETURN NULL;
    END IF;
    RETURN OLD;
END;
$$;
CREATE TRIGGER retain_current_artifact BEFORE DELETE ON events
FOR EACH ROW EXECUTE FUNCTION retain_current_artifact();
