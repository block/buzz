-- ── Relay admin actions (HTTP enforcement state machine) ──────────────────────
-- One row per HTTP report-resolution enforcement action. Tracks the durable
-- state machine from claim → enforcing → succeeded|failed|cancelled.

CREATE TABLE relay_admin_actions (
    id              UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    -- NULL for a report-less direct action (migration 0055).
    report_id       UUID,
    report_community_id UUID NOT NULL,
    -- Client-generated idempotency key (signed in NIP-98 request body).
    request_id      UUID NOT NULL,
    -- Principal who claimed the report.
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    actor_role      TEXT NOT NULL CHECK (actor_role IN ('operator', 'moderator')),
    -- The enforcement action requested.
    action          TEXT NOT NULL,
    reason          TEXT,
    -- Timeout expiration for timeout actions; NULL otherwise.
    timeout_until   TIMESTAMPTZ,
    -- Durable state machine: pending → enforcing → succeeded|failed|cancelled.
    state           TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'enforcing', 'succeeded', 'failed', 'cancelled')),
    -- Step marker: the last durably committed mutation step (NULL = none yet).
    -- Values: 'mutation_committed' (core DB mutation done), 'artifacts_done' (tombstone/notice done).
    step_marker     TEXT CHECK (step_marker IN ('mutation_committed', 'artifacts_done')),
    -- Principal who cancelled a pre-mutation failed action; NULL until cancelled.
    -- Attributes the cancel transition on the action row itself, mirroring
    -- moderation_reports.resolved_by for report resolution.
    cancelled_by    BYTEA CHECK (cancelled_by IS NULL OR length(cancelled_by) = 32),
    -- Error from the last failure, if any.
    error_message   TEXT,
    -- Per-action exclusive lease (migration 0037): fences concurrent same-request
    -- retries and lets the recovery worker claim/re-drive stranded actions.
    action_lease_token      UUID,
    action_lease_expires_at TIMESTAMPTZ,
    -- Authoritative enforcement target (migration 0047): persisted at claim time
    -- so crash-recovery can fire live side effects without re-deriving from mutable
    -- sources. enforcement_target_pubkey is the resolved target pubkey bytes for
    -- kick/ban/timeout actions; NULL for event/blob targets. enforcement_channel_id
    -- is the channel targeted by kick actions; NULL for community-wide actions.
    enforcement_target_pubkey BYTEA
        CHECK (enforcement_target_pubkey IS NULL OR length(enforcement_target_pubkey) = 32),
    enforcement_channel_id  UUID,
    -- Direct actions (migration 0055): deleted event, and requested timeout
    -- duration compared on idempotent retry.
    enforcement_target_event_id BYTEA
        CHECK (enforcement_target_event_id IS NULL OR length(enforcement_target_event_id) = 32),
    timeout_secs    BIGINT CHECK (timeout_secs IS NULL OR timeout_secs > 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Report-scoped idempotency: one action per (report, request_id).
    UNIQUE (report_community_id, report_id, request_id),
    CONSTRAINT relay_admin_actions_direct_shape CHECK (
        report_id IS NOT NULL
        OR (action = 'ban' AND enforcement_target_pubkey IS NOT NULL
            AND timeout_secs IS NULL AND timeout_until IS NULL)
        OR (action = 'timeout' AND enforcement_target_pubkey IS NOT NULL
            AND timeout_secs IS NOT NULL AND timeout_until IS NOT NULL)
        OR (action = 'delete' AND enforcement_target_event_id IS NOT NULL
            AND enforcement_target_pubkey IS NOT NULL
            AND timeout_secs IS NULL AND timeout_until IS NULL)
    ),
    FOREIGN KEY (report_community_id, report_id)
        REFERENCES moderation_reports (community_id, id)
);

CREATE INDEX idx_relay_admin_actions_report
    ON relay_admin_actions (report_community_id, report_id);
-- Direct-action idempotency (migration 0055): one per (community, request_id).
CREATE UNIQUE INDEX idx_relay_admin_actions_direct_request
    ON relay_admin_actions (report_community_id, request_id)
    WHERE report_id IS NULL;
CREATE INDEX idx_relay_admin_actions_state
    ON relay_admin_actions (state)
    WHERE state IN ('pending', 'enforcing');
-- Recovery worker (migration 0037): find stranded actions by lease expiry.
CREATE INDEX idx_relay_admin_actions_lease
    ON relay_admin_actions (action_lease_expires_at)
    WHERE state IN ('pending', 'enforcing');
