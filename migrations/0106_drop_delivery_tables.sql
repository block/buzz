-- Contract step (docs/database.md Rule 5 and "Retiring a tenant-owned table"):
-- drop the unused delivery_log and subscriptions tables and the three enum
-- types only they used (delivery_method, subscription_status, pause_reason).
-- The initial schema created them for a delivery feature that was never built.
--
-- Why this is safe:
--   * No code reads or writes either table. The release that removed every
--     reader, writer, partition-manager entry and deletion selection (#9746)
--     is the required stop: run a relay, deletion executor and operator CLI
--     build with it, or later, everywhere before this migration. A build
--     older than it still selects from these tables during community deletion
--     and user deletion, and community deletion fails its table catalog check
--     once they are gone.
--   * No foreign key references or starts at either table: 0102 and 0103
--     dropped the subscriptions keys, 0104 the delivery_log key, and the
--     delivery_log to subscriptions link was never a database constraint.
--   * Nothing ever wrote a row, so there is no data to repair first and no
--     backfill to wait for. Dropping a table does not scan it, so the drop
--     takes the same time however many rows an operator's copy holds.
--   * No other object depends on the tables or the types. The drops use
--     neither CASCADE nor a row check: an unexpected dependent fails the
--     migration instead of being dropped with the table.
--
-- Dropping the partitioned delivery_log drops its partitions, including any
-- dated monthly the relay's partition manager created at runtime before
-- #9746. Each drop takes ACCESS EXCLUSIVE on the table and, for delivery_log,
-- on every partition. The budgets end the attempt before the 1s
-- deadlock_timeout, so this transaction fails instead of queueing behind a
-- query. Retry on failure. Deployments that apply schema out of band apply
-- this file in its own transaction.
SET LOCAL lock_timeout = '300ms';
SET LOCAL statement_timeout = '800ms';

DROP TABLE IF EXISTS delivery_log;
DROP TABLE IF EXISTS subscriptions;

DROP TYPE IF EXISTS delivery_method;
DROP TYPE IF EXISTS subscription_status;
DROP TYPE IF EXISTS pause_reason;
