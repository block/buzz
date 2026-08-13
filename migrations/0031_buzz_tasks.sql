-- Buzz Tasks PR 1: owner-private event projection and search exclusion.
--
-- Rollback strategy (operator-run only): stop task event publishers/readers,
-- DROP TABLE buzz_tasks, then restore the prior search_tsv expression using
-- the same generated-column replacement pattern below. Signed task events
-- remain in `events`, so the projection can be rebuilt after a forward fix.

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

-- Kinds 44300-44302 contain plaintext owner-private titles and context. Preserve
-- the current search policy for every other kind on both fresh and brownfield
-- databases, while making task content mathematically unsearchable (`NULL @@`).
DO $$
DECLARE
    existing_expression TEXT;
BEGIN
    SELECT pg_get_expr(d.adbin, d.adrelid)
      INTO existing_expression
      FROM pg_attrdef d
      JOIN pg_attribute a
        ON a.attrelid = d.adrelid
       AND a.attnum = d.adnum
     WHERE d.adrelid = 'events'::regclass
       AND a.attname = 'search_tsv';

    IF existing_expression IS NULL THEN
        RAISE EXCEPTION 'events.search_tsv generated expression not found';
    END IF;

    ALTER TABLE events DROP COLUMN search_tsv;
    EXECUTE format(
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind IN (44300, 44301, 44302) THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
