-- Transport invalidation is generation-scoped and does not rewrite the signed
-- lease's active/tombstone state. A higher-generation replacement re-enables it.
ALTER TABLE push_leases
    ADD COLUMN endpoint_enabled BOOLEAN NOT NULL DEFAULT true;
