-- Buzz Tasks PR 1: owner-private event projection.
--
-- Rollback strategy (operator-run only): stop task event publishers/readers,
-- then DROP TABLE buzz_tasks. Signed task events remain in `events`, so the
-- projection can be rebuilt after a forward fix.
--
-- This migration deliberately does not replace events.search_tsv or rebuild
-- idx_events_search_tsv. Global search excludes kinds 44300-44302 in the
-- query layer on every database. Fresh databases also inherit the positive
-- storage allowlist from migration 0008. Any brownfield storage rewrite is a
-- separate, sized maintenance operation, never a relay-startup migration.

-- Foreign-key creation takes SHARE ROW EXCLUSIVE on the referenced parent
-- tables. Fail fast instead of extending a write pause behind a long-lived
-- production transaction; SQLx rolls the entire migration back on timeout.
SET LOCAL lock_timeout = '5s';

CREATE TABLE buzz_tasks (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    id UUID NOT NULL,
    assignee_pubkey BYTEA NOT NULL CHECK (length(assignee_pubkey) = 32),
    channel_id UUID NOT NULL,
    source_event_id BYTEA NOT NULL CHECK (length(source_event_id) = 32),
    agent_pubkey BYTEA NOT NULL CHECK (length(agent_pubkey) = 32),
    agent_name TEXT NOT NULL CHECK (octet_length(agent_name) BETWEEN 1 AND 100),
    task_type TEXT NOT NULL CHECK (task_type IN ('reply', 'approval', 'choice', 'review')),
    title TEXT NOT NULL CHECK (octet_length(title) BETWEEN 1 AND 200),
    context TEXT CHECK (context IS NULL OR octet_length(context) BETWEEN 1 AND 500),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high')),
    due_at TIMESTAMPTZ,
    status TEXT NOT NULL CHECK (status IN ('open', 'resolved', 'withdrawn')),
    source_created_at TIMESTAMPTZ NOT NULL,
    source_version BIGINT NOT NULL CHECK (source_version > 0),
    source_updated_at TIMESTAMPTZ NOT NULL,
    resolved_at TIMESTAMPTZ,
    task_event_id BYTEA NOT NULL CHECK (length(task_event_id) = 32),
    task_event_created_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels(community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, assignee_pubkey)
        REFERENCES users(community_id, pubkey) ON DELETE CASCADE,
    FOREIGN KEY (community_id, agent_pubkey)
        REFERENCES users(community_id, pubkey) ON DELETE CASCADE,
    UNIQUE (community_id, channel_id, source_event_id, assignee_pubkey),
    CHECK (
        (status = 'open' AND resolved_at IS NULL)
        OR (status IN ('resolved', 'withdrawn') AND resolved_at IS NOT NULL)
    )
);

CREATE INDEX buzz_tasks_owner_status_sort
    ON buzz_tasks (
        community_id,
        assignee_pubkey,
        status,
        priority,
        due_at,
        source_created_at DESC,
        id
    );

-- Migration 0029 installs this helper. The to_regprocedure guard keeps the
-- isolated FTS migration harness (which intentionally applies only
-- FTS-affecting migrations) usable while ensuring real upgrade chains attach
-- the same universal community write fence as every other scoped table.
DO $$
BEGIN
    IF to_regprocedure('attach_community_write_fence(regclass)') IS NOT NULL THEN
        PERFORM attach_community_write_fence('buzz_tasks');
    END IF;
END
$$;
