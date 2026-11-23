-- Who first landed each commit of a governed repository
-- (docs/spec/GitGovernance.tla, `pushed`).
--
-- The land writes one row per commit its push brings that no tip in force
-- reached, in the transaction that commits the push, after the swap; a
-- commit that already has a row keeps it. `principal` is the person the
-- push is credited to: an agent's owner, or the pusher. The land reads it
-- back so that no one approves a change that carries their own work.
--
-- A new table: no rewrite, and the deployed relay never reads or writes it.
-- IF NOT EXISTS so tests that stop migrating early can create it ahead.
SET LOCAL lock_timeout = '5s';
CREATE TABLE IF NOT EXISTS git_commit_landers (
    community_id UUID NOT NULL,
    repo_id      TEXT NOT NULL,
    owner_pubkey TEXT NOT NULL,
    commit_oid   TEXT NOT NULL,
    principal    BYTEA NOT NULL CHECK (length(principal) = 32),
    PRIMARY KEY (community_id, repo_id, owner_pubkey, commit_oid)
);
