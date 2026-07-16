-- Durable evidence of the policy version accepted when an invite claim grants
-- relay membership. Rows are scoped to the same community and member identity
-- as relay_members and are deleted with that membership.
CREATE TABLE join_policy_acceptances (
    community_id UUID NOT NULL,
    pubkey TEXT NOT NULL,
    policy_version TEXT NOT NULL CHECK (length(policy_version) = 64),
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey, policy_version),
    FOREIGN KEY (community_id, pubkey)
        REFERENCES relay_members (community_id, pubkey) ON DELETE CASCADE
);
