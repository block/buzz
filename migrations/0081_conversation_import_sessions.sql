SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- NIP-CI conversation import sessions. One row per accepted kind:42110
-- proposal; status is the relay-derived session state.
--
-- Replica-fence floor (migration 0021): imported events keep their original
-- created_at, so the relay blanks buzz.created_at_floor for each import
-- transaction only (set_config(.., true)). 0021 is checksum-frozen, so the
-- exception is recorded here. It stays sound because imports commit only
-- under an active session and only from accepts_after (activated_at plus the
-- replica proof age cap plus a margin), and a covered replica read whose
-- snapshot shows an active, unexpired session for its channel is served by
-- the writer. See replica_fence.rs and docs/database.md.
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
