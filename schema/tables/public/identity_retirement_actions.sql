-- One row per accepted operator request: audit, idempotency and result.
-- All writers use community admission; no fence trigger.
CREATE TABLE identity_retirement_actions (
    community_id    UUID NOT NULL,
    id              UUID NOT NULL,
    request_id      UUID NOT NULL,
    operation       TEXT NOT NULL CHECK (operation IN ('archive', 'unarchive', 'release')),
    target_pubkey   BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    request         JSONB NOT NULL,
    result          JSONB NOT NULL,
    published_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, request_id)
);
