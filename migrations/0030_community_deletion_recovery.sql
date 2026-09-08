-- Terminal recovery for deletion requests that have not begun irreversible
-- object deletion, plus explicit operator-global provenance ownership.
-- These ALTER TABLE statements require ACCESS EXCLUSIVE locks; fail quickly
-- rather than queueing behind long-running production transactions.
SET LOCAL lock_timeout = '5s';

ALTER TABLE community_deletion_requests
    DROP CONSTRAINT community_deletion_requests_community_id_key,
    DROP CONSTRAINT community_deletion_requests_stage_check,
    ADD CONSTRAINT community_deletion_requests_stage_check CHECK (stage IN (
        'submitted', 'inventoried', 'approved', 'fenced', 'drained',
        'bindings_removed', 'postgres_purged', 'cache_purged',
        'logically_verified', 'retention_pending', 'aborted'
    )),
    ADD COLUMN pre_quiesce_archived_at TIMESTAMPTZ,
    ADD COLUMN quiescing_started_at TIMESTAMPTZ,
    ADD COLUMN aborted_by TEXT,
    ADD COLUMN abort_reason TEXT,
    ADD COLUMN aborted_at TIMESTAMPTZ,
    ADD CHECK ((stage = 'aborted') = (aborted_at IS NOT NULL)),
    ADD CHECK ((aborted_at IS NULL) = (aborted_by IS NULL)),
    ADD CHECK ((aborted_at IS NULL) = (abort_reason IS NULL));

-- Aborted requests remain immutable audit evidence, but must not permanently
-- consume the community's one active deletion slot. Retention-pending remains
-- active because its tombstoned community must never be re-armed.
CREATE UNIQUE INDEX community_deletion_requests_active_community
    ON community_deletion_requests (community_id)
    WHERE stage <> 'aborted';

ALTER TABLE product_feedback ALTER COLUMN community_id DROP NOT NULL;
ALTER TABLE product_feedback DROP CONSTRAINT product_feedback_community_id_fkey;
ALTER TABLE product_feedback ADD CONSTRAINT product_feedback_community_id_fkey
    FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE SET NULL;

-- These are deployment-global operator evidence. community_id is provenance,
-- not ownership, so they are neither fenced nor purged with a tenant.
DROP TRIGGER IF EXISTS community_write_fence_product_feedback ON product_feedback;
DROP TRIGGER IF EXISTS community_write_fence_rate_limit_violations ON rate_limit_violations;
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations'
    ]::TEXT[])
$$;
