-- Every kind:39002 roster writer checks the roster against canonical channel
-- membership in the application (validate_channel_roster_snapshot, from
-- #8316), and since #8611 no relay requires this trigger at startup. Retire
-- the trigger that repeats that check: it compares roster labels with the
-- stored role exactly, so it would refuse rosters that show the computed
-- `member`/`bot` label. Every relay and rollback target must run #8611 or
-- later before this migration runs; older relays refuse to start without the
-- trigger. Published migrations remain checksum-frozen.
--
-- DROP TRIGGER takes ACCESS EXCLUSIVE on each partition of events in turn,
-- and keeps each lock while it waits for the next. While the locks are queued
-- or held, event reads and writes both block.
-- lock_timeout bounds each lock wait and statement_timeout bounds each whole
-- statement, as in 0049. Either timeout rolls the migration back with no
-- change, and it can be retried.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

DROP TRIGGER trg_events_guard_channel_roster_snapshot ON events;
DROP FUNCTION guard_channel_roster_snapshot();
