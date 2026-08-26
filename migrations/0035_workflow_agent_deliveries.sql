-- Durable, target-scoped delivery inbox and complete transition state machine
-- for workflow messages addressed to managed agents.
--
-- This is the DB-layer complement of the zero-I/O `workflow_delivery` protocol
-- vocabulary in buzz-core. It persists exactly one canonical binding per
-- (community, run, step, target) and owns the delivery lifecycle:
--
--     pending --claim--> claimed --finish--> finished | failed
--        ^                   |
--        +------ reap --------+   (expired lease reclaimed; prior holder fenced)
--
-- Leases are fenced by a monotonic `lease_generation`: every claim/reclaim
-- bumps it, and renew/finish only advance a row whose generation still matches
-- the caller's, so a reaped or superseded holder always fails closed. The same
-- fleet-wide reaper filters candidate rows through `community_write_allowed`,
-- exactly like the scheduler prune scan, so a quiescing/fenced/deleted tenant
-- is skipped before its write-fence trigger can abort healthy tenants.
--
-- The producer/runtime/API/ACP nodes are intentionally not reachable here.

CREATE TYPE workflow_agent_delivery_status AS ENUM (
    'pending', 'claimed', 'finished', 'failed'
);

CREATE TABLE workflow_agent_deliveries (
    community_id UUID NOT NULL REFERENCES communities(id),
    id UUID NOT NULL,
    workflow_id UUID NOT NULL,
    run_id UUID NOT NULL,
    step_id VARCHAR(64) NOT NULL CHECK (length(btrim(step_id)) > 0),
    target_pubkey BYTEA NOT NULL CHECK (octet_length(target_pubkey) = 32),
    definition_event_id BYTEA NOT NULL CHECK (octet_length(definition_event_id) = 32),
    message_event_id BYTEA NOT NULL CHECK (octet_length(message_event_id) = 32),
    message_event_created_at TIMESTAMPTZ NOT NULL,
    -- Canonical trigger authority identity (buzz-core WorkflowDeliveryCause).
    -- Exactly one identity column is populated per row; the CHECK below makes
    -- an ambiguous or absent cause unrepresentable.
    cause_kind TEXT NOT NULL CHECK (cause_kind IN ('event', 'schedule', 'webhook')),
    cause_event_id BYTEA CHECK (cause_event_id IS NULL OR octet_length(cause_event_id) = 32),
    cause_scheduled_for TIMESTAMPTZ,
    cause_webhook_invocation_id UUID,
    status workflow_agent_delivery_status NOT NULL DEFAULT 'pending',
    -- Monotonic fence token. Bumped on every claim and every reap so a stale
    -- holder's renew/finish matches zero rows and fails closed.
    lease_generation BIGINT NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
    -- Lease expiry for the current claim; NULL unless status = 'claimed'.
    lease_until TIMESTAMPTZ,
    claimed_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, run_id, step_id, target_pubkey),
    FOREIGN KEY (community_id, workflow_id) REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id) REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, message_event_created_at, message_event_id)
        REFERENCES events (community_id, created_at, id) ON DELETE CASCADE,
    -- Exactly one cause identity is present, matching cause_kind. Any other
    -- combination is a malformed authority and cannot be inserted.
    CHECK (
        (cause_kind = 'event'
            AND cause_event_id IS NOT NULL
            AND cause_scheduled_for IS NULL
            AND cause_webhook_invocation_id IS NULL)
     OR (cause_kind = 'schedule'
            AND cause_event_id IS NULL
            AND cause_scheduled_for IS NOT NULL
            AND cause_webhook_invocation_id IS NULL)
     OR (cause_kind = 'webhook'
            AND cause_event_id IS NULL
            AND cause_scheduled_for IS NULL
            AND cause_webhook_invocation_id IS NOT NULL)
    ),
    -- A lease exists iff the row is currently claimed.
    CHECK ((status = 'claimed') = (lease_until IS NOT NULL)),
    CHECK ((status = 'claimed') = (claimed_at IS NOT NULL)),
    -- Terminal rows record when they settled; non-terminal rows never do.
    CHECK ((status IN ('finished', 'failed')) = (finished_at IS NOT NULL))
);

-- Oldest-first polling of claimable work, per authenticated target.
CREATE INDEX idx_workflow_agent_deliveries_pending
    ON workflow_agent_deliveries (community_id, target_pubkey, created_at)
    WHERE status = 'pending';

-- Bounded reaper scan: only currently-leased rows can expire.
CREATE INDEX idx_workflow_agent_deliveries_lease
    ON workflow_agent_deliveries (lease_until)
    WHERE status = 'claimed';

SELECT attach_community_write_fence('workflow_agent_deliveries');
