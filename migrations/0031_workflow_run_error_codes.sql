-- Stable workflow failure classification, kept separate from redacted human diagnostics.
-- TEXT is intentional: new codes remain additive across rolling upgrades.
SET LOCAL lock_timeout = '5s';

ALTER TABLE workflow_runs ADD COLUMN error_code TEXT;

UPDATE workflow_runs
SET error_code = 'legacy_unclassified'
WHERE status IN ('failed', 'cancelled')
  AND error_code IS NULL;
