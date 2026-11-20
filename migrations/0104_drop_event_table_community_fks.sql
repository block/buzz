-- Contract step (docs/database.md Rule 2): drop the community_id foreign keys
-- on the partitioned events and delivery_log tables, and with them the clone
-- on every partition. Event writers prove the community in AdmittedTx;
-- delivery_log has no production writer, and community deletion purges both.
--
-- Required stop: as for 0102.
--
-- Each drop takes ACCESS EXCLUSIVE on the parent, on every partition and on
-- communities. LOCK TABLE without ONLY takes the partitions too, in one
-- statement, with communities last. The budgets end the attempt well before
-- the 1s deadlock_timeout, so this transaction fails instead of a relay query.
-- Retry on failure. Deployments that apply schema out of band apply this file
-- in its own transaction, apart from 0102 and 0103.
SET LOCAL lock_timeout = '300ms';
SET LOCAL statement_timeout = '800ms';

LOCK TABLE events, delivery_log, communities IN ACCESS EXCLUSIVE MODE;

ALTER TABLE events DROP CONSTRAINT events_community_id_fkey;
ALTER TABLE delivery_log DROP CONSTRAINT delivery_log_community_id_fkey;
