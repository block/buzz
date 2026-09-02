ALTER TABLE project_state_heads
    ADD COLUMN projected_revision BIGINT NOT NULL DEFAULT 0
        CHECK (projected_revision >= 0 AND projected_revision <= revision),
    ADD COLUMN projection_pubkey BYTEA
        CHECK (projection_pubkey IS NULL OR octet_length(projection_pubkey) = 32);
