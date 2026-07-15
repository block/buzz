CREATE TABLE invite_terms_acceptances (
    receipt_id UUID NOT NULL,
    community_id UUID NOT NULL REFERENCES communities(id),
    pubkey TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, receipt_id)
);
CREATE INDEX idx_invite_terms_acceptances_member
    ON invite_terms_acceptances (community_id, pubkey, accepted_at DESC);
