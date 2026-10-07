-- Relay arrival time of the latest reply anywhere under a root (set on the
-- root's row only). Read state skips a followed thread when this is not after
-- the reader's position, so a caught-up sidebar does not probe every thread
-- the reader has ever been in. last_reply_at cannot do this: it is set on the
-- direct parent, not the root.
ALTER TABLE thread_metadata ADD COLUMN last_reply_received_at TIMESTAMPTZ;

UPDATE thread_metadata root SET last_reply_received_at = r.arrival
FROM (
    SELECT tm.community_id, tm.root_event_id, max(e.received_at) AS arrival
    FROM thread_metadata tm JOIN events e ON e.community_id = tm.community_id
        AND e.created_at = tm.event_created_at AND e.id = tm.event_id
    WHERE tm.root_event_id IS NOT NULL AND tm.root_event_id <> tm.event_id
    GROUP BY tm.community_id, tm.root_event_id
) r
WHERE root.community_id = r.community_id AND root.event_id = r.root_event_id
    AND root.depth = 0;
