-- Exclude kind 31234 (NIP-37 draft wraps) from full-text search.
--
-- NIP-37 draft wraps carry NIP-44-v2 ciphertext in `content` (or empty string
-- for deletion tombstones). Indexing ciphertext would waste storage and violate
-- the "channel/DM context lives only inside the encrypted payload" invariant
-- that makes draft wraps author-private. The relay also must not index
-- plaintext compose context through any search surface.
--
-- Conditional migration: fresh installs already have the positive FTS allowlist
-- from 0008 (kind IN (0, 9, 40002, 45001, 45003)), which excludes 31234 by
-- omission. Populated databases still on the legacy negative blocklist need
-- the column rewritten to add 31234 to the exclusion list. This migration
-- inspects the live column expression and acts accordingly:
--   - Allowlist form (ELSE NULL shape) → no-op, 31234 already unsearchable.
--   - Legacy blocklist form (ELSE to_tsvector shape) → DROP + re-ADD with 31234.
--
-- DEPLOY NOTE: the DROP + re-ADD path acquires ACCESS EXCLUSIVE on `events`
-- for the duration of the migration, blocking all reads and writes to the
-- table. The GIN index rebuild that follows is a full table scan. On large
-- deployments this migration should run during a scheduled maintenance window.
-- This is the same shape as migration 0005 and was accepted as precedent.
-- The no-op path (fresh installs) does not escalate to ACCESS EXCLUSIVE.
--
-- Final kind exclusion list after this migration (legacy path):
--   1059   = KIND_GIFT_WRAP                  (NIP-17 ciphertext)
--   30300  = KIND_EVENT_REMINDER             (AUTHOR_ONLY_KINDS — defense in depth)
--   30622  = KIND_DM_VISIBILITY              (per-viewer private hide state)
--   31234  = KIND_DRAFT                      (NIP-37: AUTHOR_ONLY_KINDS — ciphertext or tombstone)
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

LOCK TABLE events IN SHARE ROW EXCLUSIVE MODE;

DO $$
DECLARE
    expr text;
BEGIN
    SELECT pg_get_expr(adbin, adrelid) INTO expr
    FROM pg_attrdef
    WHERE adrelid = 'events'::regclass
      AND adnum = (SELECT attnum FROM pg_attribute
                   WHERE attrelid = 'events'::regclass
                     AND attname = 'search_tsv');

    -- The positive allowlist form (0008) falls through to ELSE NULL::tsvector
    -- for unlisted kinds, while the legacy blocklist uses THEN NULL::tsvector
    -- for excluded kinds and ELSE to_tsvector(...) for everything else.
    -- Detecting the allowlist shape (ELSE NULL) is robust against future kind
    -- list changes in either form.
    IF expr IS NOT NULL AND expr LIKE '%ELSE NULL%' THEN
        RAISE NOTICE '0012_draft_wrap_fts: allowlist expression detected, 31234 already excluded — no-op';
        RETURN;
    END IF;

    -- Legacy blocklist form: rewrite to add 31234 to the exclusion list.
    ALTER TABLE events DROP COLUMN search_tsv;
    ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
        CASE WHEN kind IN (1059, 30300, 30622, 31234, 44100, 44101, 44200) THEN NULL::tsvector
             ELSE to_tsvector('simple', content)
        END
    ) STORED;

    -- Recreate the GIN index dropped with the column.
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
