-- Governed Git lands (docs/spec/GitGovernanceLand.tla, protocol "row").
--
-- The repository's name row becomes the row a land locks and swaps:
--   authority_version  bumped by every repository-scoped authority write;
--                      a land reads it FOR SHARE, so no such write can commit
--                      between the land's decision and its commit.
--   channel_id         the bound channel, pinned from the first announcement
--                      that names one; afterwards only a governed rebind
--                      moves it, never a re-announcement.
--   manifest           the committed repository state (a manifest digest).
--                      The object-store pointer is a cache of it; the
--                      manifest's own parent digest tells a cache one land
--                      behind from a write this row did not make.
--
-- Constant defaults and nullable columns: no table rewrite, and the deployed
-- relay's inserts (which name their columns) keep working. Rows are filled
-- lazily by the relay, never by this migration.
SET LOCAL lock_timeout = '5s';
ALTER TABLE git_repo_names
    ADD COLUMN authority_version BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN channel_id UUID,
    ADD COLUMN manifest TEXT;
