-- Definitions created before the DB write path honored `enabled` could leave
-- YAML-disabled workflows visible to enabled-workflow list queries. Backfill
-- only explicit false values so independent runtime disables remain disabled.
UPDATE workflows
SET enabled = FALSE
WHERE jsonb_typeof(definition->'enabled') = 'boolean'
  AND definition->>'enabled' = 'false';
