-- Expand only: old writers omit both nullable columns. No backfill or foreign key.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE channels
    ADD COLUMN fork_source_channel_id UUID,
    ADD COLUMN fork_source_message_id BYTEA,
    ADD CONSTRAINT channels_fork_source_paired CHECK (
        (fork_source_channel_id IS NULL AND fork_source_message_id IS NULL) OR
        (fork_source_channel_id IS NOT NULL AND fork_source_message_id IS NOT NULL
         AND octet_length(fork_source_message_id) = 32 AND fork_source_channel_id <> id)
    ) NOT VALID;
-- Validate outside the short metadata-lock transaction in the following migration.
