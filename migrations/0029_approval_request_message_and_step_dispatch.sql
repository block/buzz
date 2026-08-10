-- WF-08 hardening: make an approval reviewable, and make post-gate side
-- effects at-most-once.
--
-- 1. workflow_approvals.request_message
--
-- The gate's prompt was only ever rendered into the kind:46010 event. The
-- approval record itself carried approver_spec and expires_at but not *what*
-- was being approved, so the Workflows panel could offer an Approve button for
-- a decision whose content the approver had never seen. For a gate whose grant
-- authorises an external send, approving blind defeats the entire mechanism.
--
-- Nullable, because rows created before this migration genuinely have no
-- recorded prompt; the UI must distinguish "no package recorded" from an empty
-- one rather than render a blank card as if it were the real thing. Kept
-- separate from `note`, which is the approver's own comment travelling the
-- other way.
--
-- 2. workflow_step_dispatches
--
-- Resumption is claimed atomically (workflow_runs CAS), which makes a run
-- resume at most once concurrently. It does NOT make an individual step's side
-- effect at-most-once across a crash: a process that emits a step's event and
-- dies before its trace row is persisted will, on recovery, re-execute that
-- step. `send_message` builds a freshly-signed event each time, so the second
-- execution is a second, distinct instruction — for the post-gate step in the
-- outreach workflow that means Hermes being told twice to send the same
-- LinkedIn message.
--
-- This table is the durable dedupe key. The executor claims
-- (community_id, run_id, step_id) before dispatching; the winner performs the
-- side effect and records the resulting event id, and any later attempt reuses
-- the recorded id instead of emitting again. The PRIMARY KEY is the whole
-- mechanism: the claim is an INSERT ... ON CONFLICT DO NOTHING, so it is
-- decided by Postgres rather than by a read.

ALTER TABLE workflow_approvals
    ADD COLUMN IF NOT EXISTS request_message TEXT;

CREATE TABLE IF NOT EXISTS workflow_step_dispatches (
    community_id UUID        NOT NULL REFERENCES communities(id),
    run_id       UUID        NOT NULL,
    step_id      VARCHAR(64) NOT NULL,
    -- NULL between claiming and completing. A row with a NULL event_id whose
    -- claimed_at is old indicates a process that died mid-dispatch; it is
    -- deliberately NOT auto-cleared, because clearing it is what would permit a
    -- duplicate external send. Operator action, not a timer.
    event_id     BYTEA,
    claimed_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, run_id, step_id),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workflow_step_dispatches_run
    ON workflow_step_dispatches (community_id, run_id);
