-- First-class task entity: durable work items that harness agents (Claude Code,
-- Codex, the ACP mesh) and humans create, update, and close inside a community.
--
-- Tasks are deliberately NOT workflows. `workflows`/`workflow_runs` model the
-- scheduled execution engine; a task is a unit of work someone (or some agent)
-- owns. The two never share a lifecycle, so they never share a table.
--
-- Relay-owned rows, not Nostr events — the same modeling choice already made
-- for `workflow_runs` and `workflow_approvals` (see `buzz-relay::api::workflows`:
-- "relay-owned database rows, not Nostr events ... without inventing synthetic
-- events"). Task reads are exposed as authorized REST reads over the
-- host-derived tenant, never as a new community path segment.
--
-- Identity model: a task creator/assignee is a `users` row, never a separate
-- agent table. Agents in Buzz *are* users — they carry a `users.agent_type`
-- and an optional `users.agent_owner_pubkey` (NIP-OA). Modeling the creator as
-- a single nullable `created_by_pubkey BYTEA` therefore covers both humans and
-- agents with one community-scoped foreign key, exactly as
-- `users.agent_owner_pubkey` already does. A dedicated `created_by_agent_id`
-- would invent a second identity space that nothing else in the schema uses.
--
-- Every key leads with `community_id`: the migration lint
-- (`scoped_primary_key_unique_and_foreign_key_constraints_lead_with_community_id`)
-- rejects any primary key, unique constraint, or foreign key on a tenant table
-- that does not, so cross-tenant lookup by bare id is unrepresentable.
SET LOCAL lock_timeout = '5s';

CREATE TABLE tasks (
    community_id       UUID        NOT NULL REFERENCES communities(id),
    id                 UUID        NOT NULL DEFAULT gen_random_uuid(),
    channel_id         UUID,
    created_by_pubkey  BYTEA,
    assignee_pubkey    BYTEA,
    parent_task_id     UUID,
    title              TEXT        NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    body               TEXT,
    status             TEXT        NOT NULL DEFAULT 'todo'
                       CHECK (status IN ('todo', 'in_progress', 'blocked', 'done', 'cancelled')),
    priority           INT         NOT NULL DEFAULT 0,
    -- Harness origin ('manual', 'claude', 'codex', 'acp', 'mesh', ...) and the
    -- originating external reference. Unconstrained TEXT is intentional, for the
    -- reason 0031 gives for `workflow_runs.error_code`: a new harness must be
    -- addable across a rolling upgrade without a schema migration.
    source             TEXT,
    source_ref         TEXT,
    due_at             TIMESTAMPTZ,
    done_at            TIMESTAMPTZ,
    archived_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    -- `done_at` is the completion timestamp, so it is set exactly when the task
    -- is done. Anything else lets a 'todo' row claim a completion time.
    CONSTRAINT chk_tasks_done_at_matches_status
        CHECK ((status = 'done') = (done_at IS NOT NULL)),
    CONSTRAINT chk_tasks_not_own_parent CHECK (parent_task_id IS DISTINCT FROM id),
    CONSTRAINT chk_tasks_created_by_len
        CHECK (created_by_pubkey IS NULL OR length(created_by_pubkey) = 32),
    CONSTRAINT chk_tasks_assignee_len
        CHECK (assignee_pubkey IS NULL OR length(assignee_pubkey) = 32),
    -- A channel-bound task names a channel in its OWN community.
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id),
    FOREIGN KEY (community_id, created_by_pubkey)
        REFERENCES users (community_id, pubkey) ON DELETE SET NULL,
    FOREIGN KEY (community_id, assignee_pubkey)
        REFERENCES users (community_id, pubkey) ON DELETE SET NULL,
    -- Subtasks die with their parent; the community purge deletes the whole
    -- tenant partition of `tasks` in one statement either way.
    FOREIGN KEY (community_id, parent_task_id)
        REFERENCES tasks (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_tasks_community_status ON tasks (community_id, status);
CREATE INDEX idx_tasks_community_assignee ON tasks (community_id, assignee_pubkey)
    WHERE assignee_pubkey IS NOT NULL;
CREATE INDEX idx_tasks_community_updated ON tasks (community_id, updated_at DESC);
CREATE INDEX idx_tasks_community_channel ON tasks (community_id, channel_id)
    WHERE channel_id IS NOT NULL;
CREATE INDEX idx_tasks_community_parent ON tasks (community_id, parent_task_id)
    WHERE parent_task_id IS NOT NULL;

-- Append-only lifecycle and comment log. Also the read model behind the future
-- human-visible task feed, which is why the feed index is (community, time)
-- rather than per-task.
CREATE TABLE task_events (
    community_id  UUID        NOT NULL REFERENCES communities(id),
    id            BIGSERIAL,
    task_id       UUID        NOT NULL,
    actor_pubkey  BYTEA,
    -- 'created', 'status_changed', 'assigned', 'commented', 'title_changed',
    -- 'summary_persisted', ... Additive TEXT for the same rolling-upgrade
    -- reason as `tasks.source`.
    action        TEXT        NOT NULL CHECK (length(action) BETWEEN 1 AND 64),
    from_status   TEXT,
    to_status     TEXT,
    body          TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    CONSTRAINT chk_task_events_actor_len
        CHECK (actor_pubkey IS NULL OR length(actor_pubkey) = 32),
    FOREIGN KEY (community_id, task_id)
        REFERENCES tasks (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, actor_pubkey)
        REFERENCES users (community_id, pubkey) ON DELETE SET NULL
);

CREATE INDEX idx_task_events_task_created ON task_events (community_id, task_id, created_at);
CREATE INDEX idx_task_events_community_created ON task_events (community_id, created_at DESC);
-- "At most one persisted summary per task" is expressible directly as a partial
-- unique index, so it is enforced in the database rather than only in the relay.
CREATE UNIQUE INDEX idx_task_events_one_summary_per_task
    ON task_events (community_id, task_id)
    WHERE action = 'summary_persisted';

-- Universal community write fence. `attach_community_write_fence` documents the
-- contract: "Future migrations must invoke this helper explicitly after
-- CREATE/ALTER introduces community_id." Without these, a fenced or
-- mid-deletion tenant could still accept task writes.
SELECT attach_community_write_fence('tasks');
SELECT attach_community_write_fence('task_events');
