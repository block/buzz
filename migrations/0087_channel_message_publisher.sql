-- Expand only: absent policy preserves existing permissions and history.
SET LOCAL lock_timeout = '5s';
ALTER TABLE channels ADD COLUMN message_publisher BYTEA
    CHECK (message_publisher IS NULL OR octet_length(message_publisher) = 32);
