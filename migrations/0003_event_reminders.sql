-- Add NIP-ER event-reminder columns and due-delivery index to the events table.
--
-- `not_before` is the reminder's scheduled delivery time (Unix seconds);
-- `delivered_at` records when the scheduler published it. Both are nullable —
-- non-reminder events leave them NULL. The partial index covers only
-- undelivered, live reminders so the scheduler's due-query stays cheap.
--
-- `events` is partitioned by RANGE (created_at); ALTER TABLE on the parent
-- cascades the columns to every partition, and CREATE INDEX on the parent
-- builds a partitioned index that propagates to each partition.
--
-- Managed by sqlx migrations.

ALTER TABLE events ADD COLUMN not_before BIGINT;
ALTER TABLE events ADD COLUMN delivered_at BIGINT;
CREATE INDEX idx_events_not_before ON events (not_before)
    WHERE not_before IS NOT NULL AND deleted_at IS NULL AND delivered_at IS NULL;
