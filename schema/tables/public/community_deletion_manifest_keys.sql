-- Frozen destructive key list, chunked out of the request row so a large
-- tenant (100k-1M objects) never materializes as one multi-hundred-MB JSONB
-- value. Rows are written once in the fenced stage, stamped `deleted_at` as
-- the executor confirms each chunk removed, and dropped at logical
-- verification. The request row keeps only per-prefix count/bytes/digest
-- summaries; the chunk stream must hash to those frozen digests.
CREATE TABLE community_deletion_manifest_keys (
    request_id UUID NOT NULL REFERENCES community_deletion_requests(id) ON DELETE CASCADE,
    chunk_no BIGINT NOT NULL CHECK (chunk_no >= 0),
    prefix TEXT NOT NULL,
    keys JSONB NOT NULL,
    deleted_at TIMESTAMPTZ,
    PRIMARY KEY (request_id, chunk_no)
);
