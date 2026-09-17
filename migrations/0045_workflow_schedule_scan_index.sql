-- Keep the global scheduled-workflow keyset scan on an ordered partial index.
-- The scheduler exhausts this index in bounded pages on every successful tick.
CREATE INDEX idx_workflows_schedule_scan
    ON workflows (created_at, community_id, id)
    WHERE status = 'active'
      AND enabled = TRUE
      AND definition->'trigger'->>'on' = 'schedule';
