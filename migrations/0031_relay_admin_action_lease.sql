-- Relay admin action lease + outbox retry support.
--
-- Adds per-action exclusive lease columns to relay_admin_actions so that:
-- (a) concurrent same-request_id retries cannot both run the mutation branch,
-- (b) the action recovery worker can claim and re-drive stranded
--     pending/enforcing actions after a process crash.
--
-- Adds attempt_count and retry_after to relay_admin_outbox so that delivery
-- failures are retryable (with backoff) rather than immediately terminal.

ALTER TABLE relay_admin_actions
    ADD COLUMN action_lease_token     UUID,
    ADD COLUMN action_lease_expires_at TIMESTAMPTZ;

-- Index for the action recovery worker: find stranded actions quickly.
CREATE INDEX idx_relay_admin_actions_lease
    ON relay_admin_actions (action_lease_expires_at)
    WHERE state IN ('pending', 'enforcing');

ALTER TABLE relay_admin_outbox
    ADD COLUMN attempt_count INT NOT NULL DEFAULT 0,
    ADD COLUMN retry_after   TIMESTAMPTZ;

-- Replace the pending-delivery index to include retry_after so the worker
-- claim query can filter efficiently.
DROP INDEX IF EXISTS idx_relay_admin_outbox_pending;
CREATE INDEX idx_relay_admin_outbox_pending
    ON relay_admin_outbox (retry_after NULLS FIRST, created_at)
    WHERE state = 'pending';
