# Audit

The append-only audit records lifecycle, proposal, preview, work-item, draft, blocked-write, and Agent recommendation events. Key events include `ANOMALY_FINDING_*`, `ACTION_PROPOSAL_*`, `WORK_ITEM_*`, `APPROVAL_DRAFT_*`, `BUSINESS_WRITE_ATTEMPT_BLOCKED`, `BUSINESS_ACTION_AUTHORIZATION_DENIED`, and `AGENT_ACTION_RECOMMENDATION_EMITTED`.

Records contain event/result, entity ID, action code, status, hash, enterprise user ID, reason code, version, trace ID, and timestamp. They omit source records, evidence bodies, notes, draft text, tokens, cookies, embed codes, session values, and credentials. A database trigger rejects update or delete of audit rows.
