CREATE FUNCTION platform_claim_approval(tenant uuid, approval_token bytea, signer bytea)
RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE approval workflow_approvals%ROWTYPE;
    spec text;
BEGIN
    SELECT * INTO approval FROM workflow_approvals
    WHERE community_id=tenant AND token=approval_token FOR UPDATE;
    -- Same trim as Rust str::trim (Unicode White_Space) in the relay's approver check.
    spec := regexp_replace(approval.approver_spec, '^[\t-\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+|[\t-\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+$', '', 'g');
    IF NOT FOUND OR approval.status <> 'pending' OR approval.expires_at <= clock_timestamp()
       OR NOT (spec IN ('', 'any') OR lower(spec) = encode(signer, 'hex'))
       OR octet_length(signer) <> 32 THEN
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
