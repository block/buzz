SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Expand the demo intake. Existing submitted rows remain valid and unchanged.
-- A matching resubmission through the new CLI prepares their dummy inventory.
ALTER TABLE entity_deletion_requests
    DROP CONSTRAINT entity_deletion_requests_status_check,
    ADD CONSTRAINT entity_deletion_requests_status_check
        CHECK (status IN ('submitted', 'inventoried', 'approved', 'running', 'retry_wait', 'completed')) NOT VALID,
    ADD COLUMN inventory_manifest JSONB,
    ADD COLUMN inventory_digest TEXT,
    ADD COLUMN lease_owner TEXT,
    ADD COLUMN lease_generation BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN lease_expires_at TIMESTAMPTZ,
    ADD COLUMN heartbeat_at TIMESTAMPTZ,
    ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN next_attempt_at TIMESTAMPTZ,
    ADD COLUMN last_error TEXT,
    ADD COLUMN completed_at TIMESTAMPTZ;
-- Validate in 0070 so the scan does not retain this transaction's DDL lock.

-- Operator-global workflow history. Retained after community deletion as
-- orchestration evidence; target_community_id is provenance, not tenant ownership.
CREATE TABLE entity_deletion_stage_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    target_community_id UUID,
    stage TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'failed', 'completed')),
    is_current BOOLEAN NOT NULL DEFAULT true,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX entity_deletion_one_current_stage
    ON entity_deletion_stage_history (request_id) WHERE is_current = true;
CREATE INDEX entity_deletion_stage_history_request_idx
    ON entity_deletion_stage_history (request_id);

INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('entity_deletion_stage_history', 'Operator-global deletion workflow history retained for recovery and audit')
ON CONFLICT (table_name) DO NOTHING;

-- Approval evidence is retained after community deletion for operator audit.
-- Each operator approves an inventory once; retries preserve the original row.
CREATE TABLE entity_deletion_approvals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    inventory_digest TEXT NOT NULL CHECK (inventory_digest ~ '^[0-9a-f]{64}$'),
    approved_by TEXT NOT NULL CHECK (approved_by = btrim(approved_by) AND octet_length(approved_by) >= 1 AND octet_length(approved_by) <= 256),
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (request_id, inventory_digest, approved_by)
);

INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('entity_deletion_approvals', 'Operator-global deletion approval history retained for audit')
ON CONFLICT (table_name) DO NOTHING;
