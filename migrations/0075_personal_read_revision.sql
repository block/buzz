-- revision counts the actor's applied read intents. Every read-state response
-- and change ping carries it, so a client can drop a response older than one
-- it has already applied. Catalog-only: a constant default needs no rewrite.
SET LOCAL lock_timeout = '1s';
ALTER TABLE personal_read_accounts ADD COLUMN revision BIGINT NOT NULL DEFAULT 0;
