-- Remove the Phase-A NIP-FI relay-side authority ledger (migrations 0041 and
-- 0042). Will+Tyler resolved 2026-09-01: OSS Buzz is stateless for identity
-- ("Buzz speaks Nostr, nothing else"). The merged NIP-FI spec v2 (PR #7214,
-- squash d4420eb47) requires the nostr_pubkey claim + NIP-42 proof
-- unconditionally; the durable ledger tables are dead code.
--
-- CASCADE handles the circular deferred FK between identity_bindings and
-- identity_lifecycle_history, and dispenses with strict drop ordering.

-- ── 0042 tables ─────────────────────────────────────────────────────────────
DROP TABLE authorization_operation_version_deltas CASCADE;
DROP TABLE authorization_operation_version_delta_manifests CASCADE;
DROP TABLE authorization_authentication_denial_attempts CASCADE;
DROP TABLE authorization_admission_results CASCADE;
DROP TABLE authorization_events CASCADE;
DROP TABLE protected_object_authority CASCADE;
DROP TABLE authorization_authority_epochs CASCADE;
DROP TABLE authorization_invalidation_floors CASCADE;
DROP TABLE authorization_invalidation_domains CASCADE;
DROP TABLE authorization_event_capacity CASCADE;

-- ── 0041 tables ─────────────────────────────────────────────────────────────
-- Circular deferred FK: identity_bindings ↔ identity_lifecycle_history.
-- DROP TABLE with CASCADE resolves it without a two-step ALTER/DROP.
DROP TABLE identity_lifecycle_selectors CASCADE;
DROP TABLE identity_lifecycle_history CASCADE;
DROP TABLE identity_bindings CASCADE;
DROP TABLE identity_enrollment_policies CASCADE;
DROP TABLE authorization_operation_receipts CASCADE;

-- ── Shared functions (0041 introduced, 0042 widened) ────────────────────────
-- Triggers were dropped with their tables above; drop functions separately.
DROP FUNCTION identity_enrollment_policy_revision_guard_v1;
DROP FUNCTION nip_fi_reject_row_mutation_v1;
DROP FUNCTION nip_fi_reject_truncate_v1;
DROP FUNCTION identity_lifecycle_lock_coordinates_v1;
DROP FUNCTION identity_bindings_insert_guard_v1;
DROP FUNCTION identity_bindings_transition_guard_v1;
DROP FUNCTION identity_lifecycle_history_insert_guard_v1;
DROP FUNCTION identity_binding_history_semantics_guard_v1;
DROP FUNCTION identity_binding_birth_eligibility_guard_v1;
DROP FUNCTION authorization_operation_receipt_history_guard_v1;
DROP FUNCTION identity_lifecycle_selector_insert_guard_v1;
DROP FUNCTION identity_lifecycle_selector_history_guard_v1;
DROP FUNCTION identity_lifecycle_transition_integrity_guard_v1;
DROP FUNCTION authorization_event_capacity_before_insert_v1;
DROP FUNCTION authorization_invalidation_domain_guard_v1;
DROP FUNCTION authorization_invalidation_floor_guard_v1;
DROP FUNCTION authorization_authority_epoch_guard_v1;
DROP FUNCTION authorization_event_capacity_guard_v1;
DROP FUNCTION protected_object_authority_guard_v1;
DROP FUNCTION authorization_denial_attempt_guard_v1;
DROP FUNCTION authorization_operation_version_delta_cardinality_guard_v1;
DROP FUNCTION authorization_admission_result_guard_v1;
DROP FUNCTION authorization_operation_receipt_event_guard_v1;

-- ── Restore community_write_fence_excluded_table to its pre-0041 body ───────
-- 0041 and 0042 each widened this function via CREATE OR REPLACE to exempt
-- the NIP-FI ledger relations from the community write fence and deletion
-- catalog. With those tables gone, revert to migration 0030's body.
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations'
    ]::TEXT[])
$$;
