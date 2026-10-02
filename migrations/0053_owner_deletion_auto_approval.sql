-- Owner-origin deletion requests may be prepared automatically by the
-- privileged one-shot deletion drain. Manual approvals retain operator
-- provenance and the existing exact inventory-digest foreign key.
SET LOCAL lock_timeout = '5s';

ALTER TABLE community_deletion_approvals
    ADD COLUMN approval_origin TEXT NOT NULL DEFAULT 'operator'
        CHECK (approval_origin IN ('operator', 'owner_automatic'));

ALTER TABLE community_deletion_requests
    DROP CONSTRAINT community_deletion_requests_retry_stage_check,
    ADD CONSTRAINT community_deletion_requests_retry_stage_check
        CHECK (retry_stage IS NULL OR retry_stage IN (
            'submitted', 'approved', 'fenced', 'drained', 'bindings_removed',
            'postgres_purged', 'cache_purged', 'logically_verified'
        ));

CREATE INDEX community_deletion_requests_owner_preparable
    ON community_deletion_requests (next_attempt_at, created_at)
    WHERE request_origin = 'owner'
      AND stage = 'submitted'
      AND blocked_at IS NULL;
