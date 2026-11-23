-- ── Additive tenant tables represented in migrations 0002/0007/0017 ──────────
-- Keep desired-state schema parity with the embedded SQLx migration path.
CREATE TABLE git_repo_names (
    community_id  UUID NOT NULL,
    repo_id       TEXT NOT NULL,
    owner_pubkey  TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Governed lands (migration 0107): see migrations/0107_git_repo_authority.sql.
    authority_version BIGINT NOT NULL DEFAULT 0,
    channel_id        UUID,
    manifest          TEXT,
    PRIMARY KEY (community_id, repo_id)
);
CREATE INDEX idx_git_repo_names_owner ON git_repo_names (community_id, owner_pubkey);
