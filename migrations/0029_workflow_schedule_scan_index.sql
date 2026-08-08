-- Support the relay's global keyset scan of enabled schedule workflows.
--
-- The scheduler scans all live communities. Including community_id makes the
-- `(created_at, community_id, id)` cursor globally unique even when two
-- communities use the same workflow UUID. The partial predicate matches the
-- scheduler query so each page can advance without sorting or rescanning
-- unrelated workflow types.
CREATE INDEX IF NOT EXISTS idx_workflows_schedule_scan
    ON workflows (created_at, community_id, id)
    WHERE status = 'active'
      AND enabled = TRUE
      AND definition->'trigger'->>'on' = 'schedule';
