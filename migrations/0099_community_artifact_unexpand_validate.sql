-- Undo 0097, step 2 of 3. Validate separately from the write-blocking DDL:
-- VALIDATE CONSTRAINT scans artifact_heads under SHARE UPDATE EXCLUSIVE, so
-- channel artifact reads and writes continue while it runs.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE artifact_heads VALIDATE CONSTRAINT artifact_heads_channel_id_not_null;
