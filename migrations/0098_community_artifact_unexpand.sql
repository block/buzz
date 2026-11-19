-- Undo 0097 (community artifact expand). The community-scoped artifact
-- feature that needed it was reverted before any release read or wrote these
-- columns, so no deployed relay depends on them. 0097 stays in place because
-- databases that auto-migrated have recorded its checksum.
--
-- Restoring NOT NULL on artifact_heads.channel_id fails if a community-scoped
-- head exists. Only a database that ran the removed operator contract script
-- can have one, and that database needs the separate recovery step described
-- in the revert of #9650 before this migration can run.
--
-- Each statement takes ACCESS EXCLUSIVE on its table. Fail the deployment
-- rather than queue relay reads and writes behind a long holder.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE artifact_heads ALTER COLUMN channel_id SET NOT NULL;

DROP INDEX artifact_heads_live_type;
DROP INDEX artifact_heads_scoped_identity;
DROP INDEX artifact_revisions_scoped_identity;

ALTER TABLE artifact_heads DROP COLUMN scope;
ALTER TABLE artifact_revisions
    DROP COLUMN scope,
    DROP COLUMN community_event,
    DROP COLUMN redacted;
ALTER TABLE communities DROP COLUMN artifact_type_policies;
