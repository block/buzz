-- Additive governance head/replay lookups. Large deployments may prebuild
-- these indexes concurrently before migration; builds here are bounded.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
DO $$
BEGIN
    IF to_regclass('public.idx_audit_governance_head') IS NULL THEN
        CREATE INDEX idx_audit_governance_head ON audit_log (community_id, (detail->>'document'), seq DESC) WHERE action = 'governance_changed';
    END IF;
    IF to_regclass('public.idx_audit_governance_command') IS NULL THEN
        CREATE INDEX idx_audit_governance_command ON audit_log (community_id, object_id, seq) WHERE action = 'governance_changed';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid IN ('public.idx_audit_governance_head'::regclass, 'public.idx_audit_governance_command'::regclass)
          AND (NOT i.indisvalid OR NOT i.indisready OR NOT i.indislive)
    ) THEN
        RAISE EXCEPTION 'governance audit indexes must be valid before serving';
    END IF;
END $$;
