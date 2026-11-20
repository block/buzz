-- Per-message read state and the NIP-RS carry-over for /buzz/v1. See
-- docs/buzz-v1-activity.md. Expand only: two nullable columns with no default,
-- so neither table is rewritten. No trigger, function, foreign key, index or
-- backfill. The previous release never reads or writes them.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- When the tagged person read this mention on its own, without reading the
-- channel timeline or thread it is in. NULL: follows the read position.
ALTER TABLE event_mentions ADD COLUMN read_at TIMESTAMPTZ;

-- When the person finished carrying their NIP-RS read state over
-- (finish_import). NULL: clients do not yet trust this account's read state.
ALTER TABLE personal_read_accounts ADD COLUMN imported_at TIMESTAMPTZ;
