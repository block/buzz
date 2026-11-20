-- Undo 0097, step 3 of 3. Requires every relay pod and job to run #9689 or a
-- later release (see 0098).
--
-- SET NOT NULL uses the check validated in 0099 and skips the table scan. The
-- drops are catalog-only. Each statement takes ACCESS EXCLUSIVE on its table;
-- fail the deployment rather than queue relay reads and writes behind a long
-- holder.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE artifact_heads ALTER COLUMN channel_id SET NOT NULL;
ALTER TABLE artifact_heads DROP CONSTRAINT artifact_heads_channel_id_not_null;

DROP INDEX artifact_heads_live_type;
DROP INDEX artifact_heads_scoped_identity;
DROP INDEX artifact_revisions_scoped_identity;

ALTER TABLE artifact_heads DROP COLUMN scope;
ALTER TABLE artifact_revisions
    DROP COLUMN scope,
    DROP COLUMN community_event,
    DROP COLUMN redacted;
ALTER TABLE communities DROP COLUMN artifact_type_policies;
