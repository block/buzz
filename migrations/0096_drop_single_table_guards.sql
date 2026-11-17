-- Drop the five single-table guard triggers and their functions. The
-- application writers now own each rule (crates/buzz-db/src/store/deletion.rs
-- and the channel writers), and serving/readiness catalog checks no longer
-- require these objects:
--
-- - channels.community_id is bound once at insert and never updated;
-- - community lifecycle changes go through one transition function;
-- - deletion request inventory freezes are one-way;
-- - deletion approvals are insert-only;
-- - manifest chunk writes stop at freeze and deleted_at is one-way.
--
-- Deploy the release that contains the application guards and the catalog
-- expand to every pod and job first; do not roll back below it.
--
-- DROP TRIGGER takes ACCESS EXCLUSIVE on each table; fail the deployment
-- rather than queue relay reads and writes behind a long holder.
SET LOCAL lock_timeout = '5s';

DROP TRIGGER trg_channels_community_id_immutable ON channels;
DROP TRIGGER communities_deletion_tombstone ON communities;
DROP TRIGGER community_deletion_request_retargeting_guard ON community_deletion_requests;
DROP TRIGGER community_deletion_approval_removal_guard ON community_deletion_approvals;
DROP TRIGGER community_deletion_manifest_keys_guard ON community_deletion_manifest_keys;

DROP FUNCTION channels_community_id_immutable();
DROP FUNCTION enforce_community_tombstone();
DROP FUNCTION prevent_community_deletion_request_retargeting();
DROP FUNCTION prevent_community_deletion_approval_removal();
DROP FUNCTION protect_community_deletion_manifest_keys();
