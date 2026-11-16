-- Per thread root, how many live descendants are signed `audience=agents`:
-- agent coordination that clients fold away, so a thread summary can report
-- the replies a person reads. A separate table, because altering the hot
-- thread_metadata table would queue behind ingestion (docs/database.md
-- Rule 6). The reply writer and deleter maintain it in their own
-- transactions; a root with no row has none. Roots written before this
-- release have no row until scripts/maintenance repairs them. Community
-- purge deletes its rows (PURGE_SCOPED_TABLES).
SET LOCAL lock_timeout = '5s';
CREATE TABLE thread_agent_reply_counts (
    community_id           UUID NOT NULL,
    root_event_id          BYTEA NOT NULL,
    agent_descendant_count INT NOT NULL DEFAULT 0,
    PRIMARY KEY (community_id, root_event_id)
);
