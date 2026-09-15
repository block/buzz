-- Existing rows and older writers retain version 1. New writers explicitly
-- select version 2; no historical hashes or links are rewritten.
ALTER TABLE audit_log
    ADD COLUMN hash_version SMALLINT NOT NULL DEFAULT 1
    CHECK (hash_version IN (1, 2));
