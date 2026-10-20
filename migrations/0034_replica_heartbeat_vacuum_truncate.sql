-- replica_heartbeat is a single-row table updated continuously by every relay
-- pod. Autovacuum heap truncation briefly takes ACCESS EXCLUSIVE on the table;
-- replaying that lock on a hot standby can cancel concurrent heartbeat reads.
-- The table cannot reclaim meaningful disk space by truncating one page, so
-- disable only the truncation phase while retaining normal autovacuum cleanup.

ALTER TABLE replica_heartbeat SET (vacuum_truncate = false);
