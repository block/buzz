-- ── Thread agent-audience reply counts ───────────────────────────────────────
-- Live descendants of a thread root signed `audience=agents`. No row means 0.

CREATE TABLE thread_agent_reply_counts (
    community_id           UUID NOT NULL,
    root_event_id          BYTEA NOT NULL,
    agent_descendant_count INT NOT NULL DEFAULT 0,
    PRIMARY KEY (community_id, root_event_id)
);
