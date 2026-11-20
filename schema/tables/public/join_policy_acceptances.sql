-- ── Join policy acceptances ──────────────────────────────────────────────────
-- Durable evidence of the policy version accepted when an invite claim grants
-- relay membership. The writer binds each acceptance to a live member in the
-- same community, and removing the member removes its acceptances in the same
-- transaction.

CREATE TABLE join_policy_acceptances (
    community_id UUID NOT NULL,
    pubkey TEXT NOT NULL,
    policy_version TEXT NOT NULL CHECK (length(policy_version) = 64),
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey, policy_version)
);
