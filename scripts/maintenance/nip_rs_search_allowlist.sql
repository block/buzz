-- OUT-OF-BAND MAINTENANCE: do not run from relay startup migrations.
--
-- This rewrites every events partition and rebuilds the partitioned GIN index.
-- Run only in a maintenance window after confirming enough free space for the
-- replacement heap/TOAST/index files plus WAL. ALTER TABLE takes ACCESS
-- EXCLUSIVE, so event reads and writes block until this transaction commits.
-- Consider combining this with the planned partition repack/reclaim operation.
--
-- The allowlist includes kind 30180 (custom emoji entries, docs/nips/NIP-CE.md).
-- A database built from migrations/0008_fresh_install_search_allowlist.sql, or
-- converted by an earlier version of this script, does not index kind 30180:
-- those rows get a NULL search_tsv and emoji prefix search returns nothing.
-- Run this script on such a database BEFORE the emoji migration rollout. A
-- database that still has the original deny-list expression already indexes
-- kind 30180 and does not need it for emoji.
--
-- Validate after COMMIT; the expected result is the single row `t`:
--   SELECT pg_get_expr(d.adbin, d.adrelid) LIKE '%30180%'
--     FROM pg_attrdef d
--     JOIN pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum
--    WHERE d.adrelid = 'events'::regclass AND a.attname = 'search_tsv';
BEGIN;
SET LOCAL lock_timeout = '5s';

ALTER TABLE events DROP COLUMN search_tsv;
ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
    CASE WHEN kind IN (0, 9, 30180, 40002, 45001, 45003)
         THEN to_tsvector('simple', content)
         ELSE NULL::tsvector
    END
) STORED;
CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);

COMMIT;
