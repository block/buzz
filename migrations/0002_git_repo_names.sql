-- ── Git repo name registry (NIP-34 kind:30617) ───────────────────────────────
-- The relay holds no persistent per-repo filesystem state: git reads/writes
-- hydrate an ephemeral bare repo from object storage per request, and writer
-- serialization is the object-store pointer CAS (docs/git-on-object-storage.md,
-- Inv_NoFork). This table is the one remaining shared-state need — repo-name
-- uniqueness — moved off local disk so the relay is stateless and can run
-- multiple replicas without a ReadWriteMany volume.
--
-- Additive migration (not folded into 0001): brownfield databases that already
-- applied the pre-PR 0001 must not see its checksum change, or sqlx aborts
-- startup with a VersionMismatch. New table + index only; no edits to existing
-- objects.
--
-- Per-community, not global: a repo name is unique within a community, matching
-- the multi-tenant invariant (community_id leads the PK). The PK enforces
-- uniqueness atomically (INSERT … ON CONFLICT), replacing the old atomic
-- `create_dir`. `owner_pubkey` distinguishes idempotent re-announce (same owner)
-- from collision (different owner), and backs the per-pubkey quota via COUNT.

CREATE TABLE git_repo_names (
    community_id  UUID NOT NULL REFERENCES communities(id),
    repo_id       TEXT NOT NULL,
    owner_pubkey  TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, repo_id)
);

-- Backs the per-pubkey repo quota: COUNT(*) WHERE community_id = $1 AND owner_pubkey = $2.
CREATE INDEX idx_git_repo_names_owner ON git_repo_names (community_id, owner_pubkey);
