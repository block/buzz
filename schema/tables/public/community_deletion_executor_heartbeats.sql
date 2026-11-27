CREATE TABLE community_deletion_executor_heartbeats (
    executor_id TEXT PRIMARY KEY,
    mode TEXT NOT NULL CHECK (mode IN ('run', 'drain', 'worker')),
    request_id UUID,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    draining BOOLEAN NOT NULL DEFAULT false,
    stopped_at TIMESTAMPTZ
);
