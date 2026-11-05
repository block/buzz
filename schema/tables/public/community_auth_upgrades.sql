-- Operator-global acceptance receipts. Retained across community deletion so
-- lost-response retries cannot reuse an old operation ID for a different target.
-- target_community_id is provenance, not tenant-owned data. No serving writer.
CREATE TABLE community_auth_upgrades (
    operation_id UUID PRIMARY KEY,
    target_community_id UUID NOT NULL,
    expected_host TEXT NOT NULL,
    actor_pubkey TEXT NOT NULL,
    expected_owner_pubkey TEXT NOT NULL,
    expected_issuer TEXT NOT NULL,
    operator_pubkey TEXT NOT NULL,
    policy_generation BIGINT NOT NULL CHECK (policy_generation > 0),
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
