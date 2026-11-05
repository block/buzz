-- Contract: drop idx_events_community_deleted. No relay query filters or
-- sorts `events` on deleted_at beyond `deleted_at IS NULL`. The planner picks
-- this index only for community-wide scans, the community deletion row count
-- and purge, because it is the smallest index led by community_id; every other
-- `events` index is also led by community_id and serves them. Every write to
-- an `events` row still maintains it, and every read that plans against
-- `events` locks it on each partition. Deployed relays are unaffected: none
-- names the index, and PostgreSQL replans any cached plan that used it.
--
-- `events` is partitioned, and PostgreSQL cannot drop a partitioned index
-- CONCURRENTLY. DROP INDEX takes ACCESS EXCLUSIVE on `events`, then on each
-- partition while still holding the parent's lock; uncontended, that takes a
-- few milliseconds. lock_timeout bounds each lock wait at 1s, so the drop
-- fails rather than queue behind a long read while writers queue behind it.
-- A wait on a partition still holds `events`, so a run of contended
-- partitions can stall `events` traffic longer: statement_timeout bounds
-- the whole statement at 5s. A failed startup migration fails the boot, and
-- the next boot retries it.
-- A deployment that applies migrations out of band runs this file the same
-- way, retrying until the lock is granted. IF EXISTS makes a retry, or an
-- index an operator already dropped, a no-op that takes no lock on `events`.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
DROP INDEX IF EXISTS public.idx_events_community_deleted;
