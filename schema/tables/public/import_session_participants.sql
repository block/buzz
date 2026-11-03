-- NIP-CI per-participant consent and counters (docs/nips/NIP-CI.md).
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
