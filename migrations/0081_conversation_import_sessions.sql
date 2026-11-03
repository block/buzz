SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- NIP-CI conversation import sessions. One row per accepted kind:42110
-- proposal; status is the relay-derived session state.
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
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, proposal_id)
);

CREATE INDEX idx_import_sessions_channel
    ON import_sessions (community_id, channel_id);
CREATE INDEX idx_import_sessions_proposer
    ON import_sessions (community_id, proposer);

-- One row per proposal participant: declared counts, latest consent and the
-- counters of accepted imported events.
CREATE TABLE import_session_participants (
    community_id UUID NOT NULL,
    proposal_id BYTEA NOT NULL,
    pubkey BYTEA NOT NULL,
    source_user TEXT NOT NULL,
    declared_messages BIGINT NOT NULL,
    declared_reactions BIGINT NOT NULL,
    consent TEXT,
    published_messages BIGINT NOT NULL DEFAULT 0,
    published_reactions BIGINT NOT NULL DEFAULT 0,
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, proposal_id, pubkey)
);
