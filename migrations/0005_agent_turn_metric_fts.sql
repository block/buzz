-- ── Exclude kind 44200 (NIP-AM Agent Turn Metrics) from full-text search ──────
-- NIP-AM events carry NIP-44 ciphertext in `content`. Indexing that ciphertext
-- would waste storage and violate the spec's "NOT index the event in any
-- full-text search" requirement.
--
-- Additive migration: previously applied files must not change checksum.
-- We must DROP the generated column and re-ADD it with the extended exclusion
-- list; ALTER COLUMN cannot change a GENERATED expression in Postgres.
--
-- Final kind exclusion list after this migration:
--   1059   = KIND_GIFT_WRAP                  (NIP-17 ciphertext)
--   30300  = KIND_EVENT_REMINDER             (AUTHOR_ONLY_KINDS — defense in depth)
--   30622  = KIND_DM_VISIBILITY              (per-viewer private hide state)
--   44100  = KIND_MEMBER_ADDED_NOTIFICATION  (p-gated membership notice)
--   44101  = KIND_MEMBER_REMOVED_NOTIFICATION (p-gated membership notice)
--   44200  = KIND_AGENT_TURN_METRIC          (NIP-AM: p-gated encrypted turn metrics)
-- Constants kept in `buzz_core::kind`; inlined here because a sqlx migration
-- is frozen SQL and cannot import the Rust constant. If a new privacy-sensitive
-- kind is added there, add a new additive migration following this pattern and
-- add a regression test in `buzz-search/tests/fts_integration.rs`.
--
-- NULL tsvector never matches `@@`, so excluded rows are storage-level
-- unsearchable.

ALTER TABLE events DROP COLUMN search_tsv;
ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
    CASE WHEN kind IN (1059, 30300, 30622, 44100, 44101, 44200) THEN NULL::tsvector
         ELSE to_tsvector('simple', content)
    END
) STORED;

-- Recreate the GIN index dropped with the column.
CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
