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
--
-- The index column order is plain ascending (retry_after, created_at); the
-- claim query's own ORDER BY carries the NULLS FIRST semantics (never-retried
-- rows, retry_after IS NULL, claim first). Postgres sorts the pending candidate
-- set correctly regardless of the index's stored null ordering, and the partial
-- WHERE state = 'pending' predicate is what makes this index selective. Keeping
-- the declaration free of NULLS FIRST lets the desired-state schema (applied via
-- pgschema, which does not emit per-key null ordering) match this catalog shape
-- exactly — see admin_schema_parity_between_desired_state_and_migrations.
DROP INDEX IF EXISTS idx_relay_admin_outbox_pending;
CREATE INDEX idx_relay_admin_outbox_pending
    ON relay_admin_outbox (retry_after, created_at)
    WHERE state = 'pending';
