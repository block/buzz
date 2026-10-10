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

CREATE FUNCTION platform_claim_approval(tenant uuid, approval_token bytea, signer bytea)
RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE approval workflow_approvals%ROWTYPE;
BEGIN
    SELECT * INTO approval FROM workflow_approvals
    WHERE community_id=tenant AND token=approval_token FOR UPDATE;
    IF NOT FOUND OR approval.status <> 'pending' OR approval.expires_at <= clock_timestamp()
       OR approval.approver_spec <> encode(signer, 'hex') OR octet_length(signer) <> 32 THEN
        RETURN false;
    END IF;
    PERFORM 1 FROM workflow_runs WHERE community_id=tenant AND id=approval.run_id
        AND workflow_id=approval.workflow_id AND status='waiting_approval'
        AND current_step=approval.step_index FOR UPDATE;
    IF NOT FOUND THEN RETURN false; END IF;
    INSERT INTO platform_approval_resume_claims (community_id, token, run_id)
        VALUES (tenant, approval_token, approval.run_id) ON CONFLICT DO NOTHING;
    IF NOT FOUND THEN RETURN false; END IF;
    UPDATE workflow_approvals SET status='granted', approver_pubkey=signer, granted_at=now()
        WHERE community_id=tenant AND token=approval_token;
    RETURN true;
END;
$$;
