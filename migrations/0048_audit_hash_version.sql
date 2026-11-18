-- Preserve existing hashes. New writers explicitly select the TLV encoding.
ALTER TABLE audit_log
    ADD COLUMN hash_version SMALLINT NOT NULL DEFAULT 1
    CHECK (hash_version IN (1, 2));
