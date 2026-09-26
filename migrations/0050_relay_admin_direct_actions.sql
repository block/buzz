-- Report-less ("direct") staff actions: ban, timeout, or delete taken from the
-- admin console without a report. A direct row has report_id NULL; the
-- composite report FK is not enforced for it (MATCH SIMPLE), and
-- report_community_id remains the tenant the action applies to.
--
-- enforcement_target_event_id: the deleted event (direct delete).
-- timeout_secs: the client-requested timeout duration, compared on idempotent
--   retry; timeout_until stays the authoritative expiry fixed at acceptance.

ALTER TABLE relay_admin_actions
    ALTER COLUMN report_id DROP NOT NULL,
    ADD COLUMN enforcement_target_event_id BYTEA
        CHECK (enforcement_target_event_id IS NULL OR length(enforcement_target_event_id) = 32),
    ADD COLUMN timeout_secs BIGINT CHECK (timeout_secs IS NULL OR timeout_secs > 0),
    ADD CONSTRAINT relay_admin_actions_direct_shape CHECK (
        report_id IS NOT NULL
        OR (action = 'ban' AND enforcement_target_pubkey IS NOT NULL
            AND timeout_secs IS NULL AND timeout_until IS NULL)
        OR (action = 'timeout' AND enforcement_target_pubkey IS NOT NULL
            AND timeout_secs IS NOT NULL AND timeout_until IS NOT NULL)
        OR (action = 'delete' AND enforcement_target_event_id IS NOT NULL
            AND enforcement_target_pubkey IS NOT NULL
            AND timeout_secs IS NULL AND timeout_until IS NULL)
    );

-- Direct-action idempotency: one action per (community, request_id).
CREATE UNIQUE INDEX idx_relay_admin_actions_direct_request
    ON relay_admin_actions (report_community_id, request_id)
    WHERE report_id IS NULL;
