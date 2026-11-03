-- NIP-CI conversation import sessions (docs/nips/NIP-CI.md).
CREATE TABLE import_sessions (
    community_id UUID NOT NULL,
    proposal_id BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    proposer BYTEA NOT NULL,
    manifest TEXT NOT NULL,
    source_type TEXT NOT NULL,
    is_dm BOOLEAN NOT NULL,
    range_from BIGINT NOT NULL,
    range_to BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    requires_admin BOOLEAN NOT NULL,
    admin_consent TEXT,
    status TEXT NOT NULL,
    activated_at BIGINT,
    accepts_after BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, proposal_id)
);
CREATE INDEX idx_import_sessions_channel
    ON import_sessions (community_id, channel_id);
CREATE INDEX idx_import_sessions_proposer
    ON import_sessions (community_id, proposer);
