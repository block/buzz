-- Durable execution hold, independent of NIP-IA visibility. Run admission
-- refuses a workflow whose owner, or whose owner's owner, has a row here.
CREATE TABLE identity_retirement_holds (
    community_id    UUID NOT NULL,
    pubkey          BYTEA NOT NULL CHECK (length(pubkey) = 32),
    action_id       UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey)
);
