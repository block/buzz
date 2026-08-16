-- Durable projection for the public agent-job event protocol (kinds 43001-43006).
-- Signed events remain collaboration truth; these rows are an atomic admission
-- index so lifecycle validation never depends on an event-history pre-query.

CREATE TABLE agent_jobs (
    community_id          UUID NOT NULL REFERENCES communities(id),
    job_id                UUID NOT NULL,
    request_event_id      BYTEA NOT NULL CHECK (length(request_event_id) = 32),
    request_created_at    TIMESTAMPTZ NOT NULL,
    channel_id            UUID NOT NULL,
    requester_pubkey      BYTEA NOT NULL CHECK (length(requester_pubkey) = 32),
    target_pubkey         BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    state                 TEXT NOT NULL CHECK (state IN (
        'requested', 'accepted', 'running', 'cancelling',
        'succeeded', 'failed', 'cancelled', 'lost'
    )),
    attempt               BIGINT NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    progress_seq          NUMERIC(20, 0),
    summary               TEXT NOT NULL,
    cancel_requested      BOOLEAN NOT NULL DEFAULT FALSE,
    cancel_event_id       BYTEA CHECK (cancel_event_id IS NULL OR length(cancel_event_id) = 32),
    terminal_event_id     BYTEA CHECK (terminal_event_id IS NULL OR length(terminal_event_id) = 32),
    terminal_created_at   TIMESTAMPTZ,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, job_id),
    UNIQUE (community_id, request_event_id)
);

CREATE INDEX idx_agent_jobs_target_state
    ON agent_jobs (community_id, target_pubkey, state, updated_at DESC, job_id);
CREATE INDEX idx_agent_jobs_requester_state
    ON agent_jobs (community_id, requester_pubkey, state, updated_at DESC, job_id);
CREATE INDEX idx_agent_jobs_channel_updated
    ON agent_jobs (community_id, channel_id, updated_at DESC, job_id);

CREATE TABLE agent_job_events (
    community_id       UUID NOT NULL REFERENCES communities(id),
    event_id           BYTEA NOT NULL CHECK (length(event_id) = 32),
    event_created_at   TIMESTAMPTZ NOT NULL,
    job_id             UUID NOT NULL,
    chain_seq          BIGINT NOT NULL CHECK (chain_seq > 0),
    kind               INT NOT NULL CHECK (kind BETWEEN 43001 AND 43006),
    author_pubkey      BYTEA NOT NULL CHECK (length(author_pubkey) = 32),
    attempt            BIGINT CHECK (attempt IS NULL OR attempt >= 0),
    progress_seq       NUMERIC(20, 0),
    PRIMARY KEY (community_id, event_id),
    UNIQUE (community_id, job_id, chain_seq),
    FOREIGN KEY (community_id, job_id)
        REFERENCES agent_jobs(community_id, job_id) ON DELETE CASCADE
);

CREATE INDEX idx_agent_job_events_chain
    ON agent_job_events (community_id, job_id, chain_seq);
