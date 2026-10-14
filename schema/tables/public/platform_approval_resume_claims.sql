-- ── Approval resume claims ────────────────────────────────────────────────────
-- A consumed approval is never replayed automatically after a crash.
CREATE TABLE platform_approval_resume_claims (
    community_id uuid NOT NULL,
    token bytea NOT NULL,
    run_id uuid NOT NULL,
    state text NOT NULL DEFAULT 'recovery_required'
        CHECK (state IN ('recovery_required', 'completed')),
    claimed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, token),
    FOREIGN KEY (community_id, token)
        REFERENCES workflow_approvals (community_id, token) ON DELETE CASCADE
);
