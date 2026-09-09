-- Purpose-built FounderPortal collaboration convergence journal. This is not a
-- generalized event bus or outbox and contains no credentials or business data.
CREATE TABLE founderportal_bridge.sync_operations (
    operation_key TEXT PRIMARY KEY CHECK (operation_key = btrim(operation_key) AND operation_key <> ''),
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND tenant_id <> ''),
    operation_type TEXT NOT NULL CHECK (operation_type IN ('ensure_community','ensure_member','revoke_member','archive_community')),
    actor_type TEXT CHECK (actor_type IS NULL OR actor_type IN ('human','agent')),
    actor_id TEXT CHECK (actor_id IS NULL OR (actor_id = btrim(actor_id) AND actor_id <> '')),
    desired_version BIGINT NOT NULL CHECK (desired_version > 0),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','running','succeeded','retryable_error','terminal_error','superseded')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 8 CHECK (max_attempts > 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    last_error_code TEXT CHECK (last_error_code IS NULL OR (last_error_code = btrim(last_error_code) AND last_error_code <> '' AND length(last_error_code) <= 128)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((operation_type IN ('ensure_member','revoke_member')) = (actor_type IS NOT NULL AND actor_id IS NOT NULL)),
    CHECK ((state = 'running') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL))
);
CREATE INDEX founderportal_sync_claim_idx ON founderportal_bridge.sync_operations (available_at, created_at) WHERE state IN ('pending','retryable_error','running');
CREATE INDEX founderportal_sync_desired_idx ON founderportal_bridge.sync_operations (tenant_id, operation_type, actor_type, actor_id, desired_version DESC);
INSERT INTO _operator_global_tables (table_name, reason) VALUES
('sync_operations', 'FounderPortal bridge-owned, tenant-scoped convergence journal');
