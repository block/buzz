-- Backfill d_tag for existing NIP-33 range events (kind 30000–39999).
-- Idempotent: only updates rows where d_tag is still NULL.
-- Includes soft-deleted rows so the column is fully populated.
-- Run once after adding the d_tag column to the events table.
--
-- Usage: psql $DATABASE_URL -f scripts/backfill-d-tag.sql

UPDATE events
SET d_tag = COALESCE(
    (SELECT elem->>1
     FROM jsonb_array_elements(tags) AS elem
     WHERE elem->>0 = 'd'
     LIMIT 1),
    ''
)
WHERE kind BETWEEN 30000 AND 39999
  AND d_tag IS NULL;
