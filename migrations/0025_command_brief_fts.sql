-- Exclude kind 44210 (NIP-CB encrypted owner-only Command Briefs) from FTS.
-- Additive only: applied migration checksums are immutable.
ALTER TABLE events DROP COLUMN search_tsv;
ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
    CASE WHEN kind IN (1059, 30300, 30350, 30622, 44100, 44101, 44200, 44210)
         THEN NULL::tsvector
         ELSE to_tsvector('simple', content)
    END
) STORED;
CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
