SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

ALTER TABLE channel_members
    ADD COLUMN auto_join_token UUID,
    ADD COLUMN auto_join_version TIMESTAMPTZ;

CREATE TABLE relay_invite_claims (
    community_id UUID NOT NULL,
    id UUID NOT NULL DEFAULT gen_random_uuid(),
    queue_order BIGINT GENERATED ALWAYS AS IDENTITY,
    request JSONB NOT NULL,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (clock_timestamp() + interval '12 seconds'),
    completed_at TIMESTAMPTZ,
    result JSONB,
    PRIMARY KEY (community_id, id),
    CHECK (completed_at IS NULL OR result IS NOT NULL)
);

CREATE INDEX idx_relay_invite_claims_pending
    ON relay_invite_claims (community_id, queue_order, id)
    WHERE completed_at IS NULL;
CREATE INDEX idx_relay_invite_claims_expiry
    ON relay_invite_claims (expires_at, community_id) WHERE result IS NULL;
CREATE INDEX idx_relay_invite_claims_admitted_expiry
    ON relay_invite_claims (expires_at, community_id)
    WHERE result IS NOT NULL AND completed_at IS NULL;
CREATE INDEX idx_relay_invite_claims_completed
    ON relay_invite_claims (completed_at, community_id) WHERE completed_at IS NOT NULL;
