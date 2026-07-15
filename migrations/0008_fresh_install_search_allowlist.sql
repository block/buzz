-- Give new, empty installations the positive FTS allowlist without rewriting
-- populated databases during relay startup. Existing installations keep their
-- current search_tsv expression until an operator runs the sized out-of-band
-- maintenance script in scripts/maintenance/nip_rs_search_allowlist.sql.
--
-- Serialize the emptiness check with event writers. Reads remain available on
-- populated databases; an actually empty table upgrades briefly to ACCESS
-- EXCLUSIVE for the generated-column replacement and index build.
LOCK TABLE events IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM events LIMIT 1) THEN
        ALTER TABLE events DROP COLUMN search_tsv;
        ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
            CASE WHEN kind IN (0, 9, 40002, 45001, 45003)
                 THEN to_tsvector('simple', content)
                 ELSE NULL::tsvector
            END
        ) STORED;
        CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
    END IF;
END $$;
