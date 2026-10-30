-- Keep the deletion control plane writable while its target tenant is fenced.
-- This predicate is the single SQL source of truth used by attachment and live
-- catalog validation.
CREATE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations', 'operator_listener_outbox'
    ]::TEXT[])
$$;
