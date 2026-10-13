-- A frontier records the message it was read through, so the sidebar can
-- report it. NULL for frontiers written before this column. Channel and thread
-- frontiers are independent: the whole-channel cut covering every thread is
-- gone.
ALTER TABLE personal_read_frontiers
    ADD COLUMN through_message_id BYTEA CHECK (octet_length(through_message_id) = 32),
    DROP COLUMN threads_through_timestamp;
