-- Replica heartbeat: a portable read-side freshness observation for the
-- replica fence (see crates/buzz-db/src/replica_fence.rs).
--
-- Why: the fence's ordered writer-side proof previously ended in a WAL-LSN
-- comparison (`pg_last_wal_replay_lsn() >= L`), which Aurora's reader
-- endpoints do not expose — the fence therefore never opened on Aurora, by
-- design (fail closed). This table replaces only that read-side observation:
-- the probe commits a monotonically increasing `token` AFTER the ordered
-- writer scan (clock sample -> oldest-xact guard), and a reader session that
-- observes token >= M has, by WAL/storage replay order, also replayed every
-- commit that preceded M. The commit-time floor guard (migration 0021) and
-- the writer-side scan remain load-bearing and unchanged.
--
-- Shape: exactly one row, enforced by the CHECK'd primary key. Every relay
-- pod's probe increments the same row; the single-row UPDATE is the
-- serialization point that makes tokens globally commit-ordered, which is
-- what lets a pod prove coverage from the greatest token it retained that is
-- <= the token a reader session observes (multi-pod safety).
--
-- `epoch` detects resets: a restore/re-seed that rolls `token` backwards
-- must never let a stale retained token masquerade as fresh coverage.
-- Readers validate the observed epoch against the epoch retained with each
-- token; a mismatch fails closed (route to writer).
--
-- Not an events row: exempt from the created_at floor guard by construction,
-- and deliberately deployment-global (no community_id) — it describes the
-- replication topology, not tenant data.

CREATE TABLE replica_heartbeat (
    id    smallint PRIMARY KEY CHECK (id = 1),
    epoch uuid     NOT NULL DEFAULT gen_random_uuid(),
    token bigint   NOT NULL DEFAULT 0
);

INSERT INTO replica_heartbeat (id) VALUES (1);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('replica_heartbeat', 'single-row replication freshness token; describes deployment topology, never tenant data');
