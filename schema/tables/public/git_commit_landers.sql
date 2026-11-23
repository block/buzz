-- Who first landed each commit of a governed repository: see
-- migrations/0108_git_commit_landers.sql.
CREATE TABLE git_commit_landers (
    community_id UUID NOT NULL,
    repo_id      TEXT NOT NULL,
    owner_pubkey TEXT NOT NULL,
    commit_oid   TEXT NOT NULL,
    principal    BYTEA NOT NULL CHECK (length(principal) = 32),
    PRIMARY KEY (community_id, repo_id, owner_pubkey, commit_oid)
);
