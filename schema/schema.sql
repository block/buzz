-- Buzz initial Postgres schema — multi-tenant.
--
-- Source of truth for fresh database setup. This is a clean, from-scratch
-- schema in which `community_id` is a first-class, server-resolved key on
-- every tenant-scoped row. It is NOT additive over the single-community
-- schema; the rewrite replaces it. Existing single-community deployments
-- migrate via the documented backfill migration (0002), which assigns all
-- pre-existing rows to one default community.
--
-- The governing contract is docs/multi-tenant-conformance.md. Every table
-- below cites the conformance surface it implements. The invariant behind the
-- whole schema (conformance "row zero"): a request's community is resolved
-- from the connection host by the server, never supplied by the client, and
-- every scoped row carries that immutable `community_id`.
--
-- Migration-lint obligations enforced by the Lane 0 lint harness:
--   1. Every tenant-scoped table has `community_id NOT NULL`.
--   2. No UNIQUE / PRIMARY KEY / FK on a scoped table is observable across
--      communities: each leads with `community_id` (or, for child rows whose
--      parent already pins the community, joins carry the community tuple).
--   3. `channels.community_id` is immutable (trigger below; no UPDATE path).
--   4. Operator-global tables are named in the explicit allowlist, not implied.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ── Custom types ──────────────────────────────────────────────────────────────

CREATE TYPE channel_type AS ENUM ('stream', 'forum', 'dm', 'workflow');
CREATE TYPE channel_visibility AS ENUM ('open', 'private');
CREATE TYPE member_role AS ENUM ('owner', 'admin', 'member', 'guest', 'bot');
CREATE TYPE workflow_status AS ENUM ('active', 'disabled', 'archived');
CREATE TYPE run_status AS ENUM ('pending', 'running', 'waiting_approval', 'completed', 'failed', 'cancelled');
CREATE TYPE approval_status AS ENUM ('pending', 'granted', 'denied', 'expired');
CREATE TYPE delivery_method AS ENUM ('webhook', 'websocket');
CREATE TYPE subscription_status AS ENUM ('active', 'paused', 'deleted');
CREATE TYPE pause_reason AS ENUM ('user', 'system', 'rate_limit');
CREATE TYPE channel_add_policy AS ENUM ('anyone', 'owner_only', 'nobody');

-- ── Communities ───────────────────────────────────────────────────────────────
-- Conformance: row zero (host binding). The host map. `resolve_host(host)`
-- reads exactly one row here to mint the request's TenantContext. This table
-- is OPERATOR-GLOBAL: it is the registry of tenants, not itself tenant-scoped,
-- so it carries no `community_id` of its own (its `id` IS the community key).
-- Listed in the lint allowlist as operator-global.
--
-- Host normalization (Lane 0 contract): `host` is stored already-normalized —
-- ASCII-lowercased, trailing dot stripped, default port omitted. The UNIQUE is
-- on `lower(host)` belt-and-suspenders so `Relay.Example` and `relay.example`
-- can never become two tenants even if a writer forgets to normalize.
-- `resolve_host()` (buzz-core) applies the identical normalization before
-- lookup, so resolution and storage agree by construction.

CREATE TABLE communities (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    host            VARCHAR(255) NOT NULL,
    signing_key     BYTEA,
    -- Per-community workspace icon (NIP-11 `icon`), set via kind:9033.
    -- Added by migration 0003; kept here so desired-state applies match.
    icon            TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at     TIMESTAMPTZ,
    deletion_state  TEXT NOT NULL DEFAULT 'active' CHECK (deletion_state IN ('active', 'quiescing', 'fenced', 'tombstone')),
    deletion_fence_generation BIGINT NOT NULL DEFAULT 0 CHECK (deletion_fence_generation >= 0),
    deleted_at      TIMESTAMPTZ,
    CONSTRAINT chk_communities_id_not_nil CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

CREATE UNIQUE INDEX idx_communities_host ON communities (lower(host));

-- ── Channels ──────────────────────────────────────────────────────────────────
-- Conformance: "Channels and channel membership". `community_id` immutable.
-- Channel UUIDs stay valid wire identifiers, but they are NOT globally unique:
-- the PK is `(community_id, id)`, so the same UUID may legitimately exist in two
-- communities (conformance lists "same channel UUID collision in two
-- communities" as a required isolation test). Handlers always carry `ctx`, so
-- `(ctx.community, h)` names exactly one channel; a client-supplied `h` can
-- never reach another community's channel.

CREATE TABLE channels (
    id              UUID NOT NULL DEFAULT gen_random_uuid(),
    community_id    UUID NOT NULL REFERENCES communities(id),
    name            VARCHAR(255) NOT NULL,
    channel_type    channel_type NOT NULL DEFAULT 'stream',
    visibility      channel_visibility NOT NULL DEFAULT 'open',
    description     TEXT,
    canvas          TEXT,
    created_by      BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at     TIMESTAMPTZ,
    deleted_at      TIMESTAMPTZ,
    nip29_group_id  VARCHAR(255),
    topic_required  BOOLEAN NOT NULL DEFAULT FALSE,
    max_members     INT,
    topic           TEXT,
    topic_set_by    BYTEA,
    topic_set_at    TIMESTAMPTZ,
    purpose         TEXT,
    purpose_set_by  BYTEA,
    purpose_set_at  TIMESTAMPTZ,
    participant_hash BYTEA,
    ttl_seconds     INT,
    ttl_deadline    TIMESTAMPTZ,
    PRIMARY KEY (community_id, id),
    CONSTRAINT chk_channels_id_not_nil CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

-- nip29 group id and DM participant hash are unique WITHIN a community, not globally.
CREATE UNIQUE INDEX idx_channels_nip29_group ON channels (community_id, nip29_group_id)
    WHERE nip29_group_id IS NOT NULL;
CREATE UNIQUE INDEX idx_channels_dm_hash ON channels (community_id, participant_hash)
    WHERE participant_hash IS NOT NULL;
CREATE INDEX idx_channels_community_type ON channels (community_id, channel_type);
CREATE INDEX idx_channels_community_visibility ON channels (community_id, visibility);
CREATE INDEX idx_channels_created_by ON channels (community_id, created_by);
CREATE INDEX idx_channels_ttl_expiry ON channels (ttl_deadline)
    WHERE ttl_seconds IS NOT NULL AND archived_at IS NULL AND deleted_at IS NULL;
-- Tenant-independent channel-id → community lookups (Db::communities_of_channels,
-- Db::community_of_channel) carry no community_id predicate, so no
-- community_id-leading index can serve them. Covering + partial: index-only scan.
-- Not UNIQUE — the same channel id may exist under more than one community.
CREATE INDEX idx_channels_id_live ON channels (id) INCLUDE (community_id)
    WHERE deleted_at IS NULL;

-- channels.community_id is immutable: a channel can never be re-tenanted.
-- (Conformance: "Migration lint forbids channel re-tenanting except through an
-- explicitly modeled admission path." We have no such path, so: hard block.)
CREATE FUNCTION channels_community_id_immutable() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id THEN
        RAISE EXCEPTION 'channels.community_id is immutable (channel % cannot be re-tenanted)', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_channels_community_id_immutable
    BEFORE UPDATE ON channels
    FOR EACH ROW EXECUTE FUNCTION channels_community_id_immutable();

-- ── Channel members ───────────────────────────────────────────────────────────
-- Conformance: "Channels and channel membership". PK leads with community_id.

CREATE TABLE channel_members (
    community_id UUID NOT NULL REFERENCES communities(id),
    channel_id  UUID NOT NULL,
    pubkey      BYTEA NOT NULL,
    role        member_role NOT NULL DEFAULT 'member',
    joined_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invited_by  BYTEA,
    removed_at  TIMESTAMPTZ,
    removed_by  BYTEA,
    hidden_at   TIMESTAMPTZ,
    PRIMARY KEY (community_id, channel_id, pubkey),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_channel_members_pubkey ON channel_members (community_id, pubkey)
    WHERE removed_at IS NULL;

-- ── Users ─────────────────────────────────────────────────────────────────────
-- Conformance: "Users, profiles, NIP-05, and user search". One profile per
-- (community, pubkey): the same key reposts kind:0 in each community it joins.

CREATE TABLE users (
    community_id        UUID NOT NULL REFERENCES communities(id),
    pubkey              BYTEA NOT NULL,
    nip05_handle        VARCHAR(255),
    display_name        VARCHAR(255),
    avatar_url          TEXT,
    about               TEXT,
    agent_type          VARCHAR(255),
    capabilities        JSONB,
    okta_user_id        VARCHAR(255),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deactivated_at      TIMESTAMPTZ,
    metadata_event_id   BYTEA,
    agent_owner_pubkey  BYTEA,
    channel_add_policy  channel_add_policy NOT NULL DEFAULT 'anyone',
    PRIMARY KEY (community_id, pubkey),
    CONSTRAINT chk_users_pubkey_len CHECK (LENGTH(pubkey) = 32),
    -- agent owner is a user in the SAME community.
    FOREIGN KEY (community_id, agent_owner_pubkey)
        REFERENCES users (community_id, pubkey) ON DELETE SET NULL
);

-- NIP-05 handle and Okta id unique within a community, not globally.
CREATE UNIQUE INDEX idx_users_nip05 ON users (community_id, lower(nip05_handle))
    WHERE nip05_handle IS NOT NULL;
CREATE UNIQUE INDEX idx_users_okta ON users (community_id, okta_user_id)
    WHERE okta_user_id IS NOT NULL;

-- ── Events (partitioned by month on created_at) ──────────────────────────────
-- Conformance: "Channel-less global events and DMs". `community_id` leads the
-- PK and every hot-path index. Partition stays BY RANGE (created_at) — the
-- monthly partition manager is unchanged (Max's call, plan §5/Lane0 contract).
-- Cross-community dedup: same signed event may exist in two communities;
-- (community_id, created_at, id) dedupes within one, allows across.

CREATE TABLE events (
    community_id UUID NOT NULL REFERENCES communities(id),
    id          BYTEA NOT NULL,
    pubkey      BYTEA NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    kind        INT NOT NULL,
    tags        JSONB NOT NULL,
    content     TEXT NOT NULL,
    -- Full-text search vector (Typesense → Postgres FTS). Generated/STORED so
    -- it is a single source of truth — no sidecar indexer to keep coherent
    -- (Quinn option A, Lane-0 call). 'simple' config = no stemming/stopwords,
    -- matching the existing substring-ish search semantics; the search lane can
    -- revisit the config behind evidence. Tenant scoping is by the
    -- community-leading btree filters BitmapAnd-ed with the GIN probe, so the
    -- GIN index itself stays the minimal `GIN (search_tsv)` (Max's caveat:
    -- avoid btree_gin unless EXPLAIN proves it buys something).
    -- Privacy: encrypted/private routing wrappers and p-gated membership notices
    -- must never be discoverable through NIP-50 full-text search. NULL tsvector
    -- never matches `@@`.
    -- Keep in sync with migrations (final state: 0001 + 0005 + 0014 + 0033).
    search_tsv  TSVECTOR GENERATED ALWAYS AS (
        CASE WHEN kind IN (1059, 30179, 30300, 30350, 30622, 44100, 44101, 44200) THEN NULL::tsvector
             ELSE to_tsvector('simple', content)
        END
    ) STORED,
    sig         BYTEA NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    channel_id  UUID,
    deleted_at  TIMESTAMPTZ,
    d_tag       TEXT,
    not_before  BIGINT,
    delivered_at BIGINT,
    PRIMARY KEY (community_id, created_at, id)
) PARTITION BY RANGE (created_at);

CREATE TABLE events_p_past PARTITION OF events
    FOR VALUES FROM (MINVALUE) TO ('2026-01-01');
CREATE TABLE events_p2026_01 PARTITION OF events
    FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
CREATE TABLE events_p2026_02 PARTITION OF events
    FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
CREATE TABLE events_p2026_03 PARTITION OF events
    FOR VALUES FROM ('2026-03-01') TO ('2026-04-01');
CREATE TABLE events_p2026_04 PARTITION OF events
    FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');
CREATE TABLE events_p2026_05 PARTITION OF events
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE events_p2026_06 PARTITION OF events
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE events_p_future PARTITION OF events
    FOR VALUES FROM ('2026-07-01') TO (MAXVALUE);

-- Direct id lookup: the PK can't serve `WHERE id=$1` because created_at sits
-- between community_id and id. This index makes the scoped form
-- `WHERE community_id=$ AND id=$` index-served, not a partition scan.
CREATE INDEX idx_events_community_id ON events (community_id, id, created_at DESC);
-- Hot-path indexes, all community-leading.
CREATE INDEX idx_events_community_channel_created
    ON events (community_id, channel_id, created_at DESC, id);
CREATE INDEX idx_events_community_pubkey_kind_created
    ON events (community_id, pubkey, kind, created_at DESC, id);
CREATE INDEX idx_events_community_kind_created
    ON events (community_id, kind, created_at DESC, id);
CREATE INDEX idx_events_community_deleted ON events (community_id, deleted_at);
-- Addressable (replaceable) and NIP-33 parameterized lookups.
CREATE INDEX idx_events_addressable
    ON events (community_id, kind, pubkey, channel_id, deleted_at);
CREATE INDEX idx_events_parameterized
    ON events (community_id, kind, pubkey, d_tag, created_at DESC, id)
    WHERE d_tag IS NOT NULL AND deleted_at IS NULL;
CREATE INDEX idx_events_not_before ON events (community_id, not_before)
    WHERE not_before IS NOT NULL AND deleted_at IS NULL AND delivered_at IS NULL;
-- Full-text search. Minimal GIN over the generated tsvector; community scoping
-- is supplied by the community-leading btree filters above (BitmapAnd), so this
-- stays a single-column GIN. The search lane confirms the final spelling with
-- EXPLAIN before its work lands (Quinn option A; Max's index-spelling caveat).
CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);

-- ── Event mentions ────────────────────────────────────────────────────────────
-- Conformance: "Channel-less global events and DMs" (#p fan-out). The join to
-- events MUST carry the community tuple (e.community_id = m.community_id AND
-- e.id = m.event_id) — bare e.id = m.event_id would leak cross-community
-- mentions (Max, verified at event.rs:222).

CREATE TABLE event_mentions (
    community_id        UUID NOT NULL REFERENCES communities(id),
    pubkey_hex          VARCHAR(64) NOT NULL,
    event_id            BYTEA NOT NULL,
    event_created_at    TIMESTAMPTZ NOT NULL,
    channel_id          UUID,
    event_kind          INT,
    PRIMARY KEY (community_id, pubkey_hex, event_id)
);

CREATE INDEX idx_event_mentions_pubkey_created
    ON event_mentions (community_id, pubkey_hex, event_created_at DESC);
CREATE INDEX idx_event_mentions_pubkey_kind_created
    ON event_mentions (community_id, pubkey_hex, event_kind, event_created_at DESC);

-- ── Subscriptions ─────────────────────────────────────────────────────────────
-- Conformance: "Mesh, agents, ACP/MCP, and CLI" (persisted subscriptions).

CREATE TABLE subscriptions (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  VARCHAR(255) NOT NULL,
    owner_pubkey        BYTEA NOT NULL,
    filter_kinds        JSONB,
    filter_authors      JSONB,
    filter_channel_ids  JSONB,
    filter_since        TIMESTAMPTZ,
    filter_until        TIMESTAMPTZ,
    delivery_method     delivery_method NOT NULL DEFAULT 'webhook',
    delivery_url        TEXT,
    status              subscription_status NOT NULL DEFAULT 'active',
    pause_reason        pause_reason,
    delivered_count     BIGINT NOT NULL DEFAULT 0,
    error_count         BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey)
);

-- ── Delivery log (partitioned by month on delivered_at) ──────────────────────
-- Conformance: subscription delivery audit. community_id carried for tenant
-- attribution; child of subscriptions.

CREATE TABLE delivery_log (
    community_id    UUID NOT NULL REFERENCES communities(id),
    id              BIGINT GENERATED ALWAYS AS IDENTITY,
    subscription_id VARCHAR(255),
    event_id        BYTEA,
    method          delivery_method,
    delivered_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success         BOOLEAN,
    http_status     INT,
    error_message   TEXT,
    attempt_number  INT DEFAULT 1,
    PRIMARY KEY (delivered_at, id)
) PARTITION BY RANGE (delivered_at);

CREATE TABLE delivery_log_p_past PARTITION OF delivery_log
    FOR VALUES FROM (MINVALUE) TO ('2026-03-01');
CREATE TABLE delivery_log_p2026_03 PARTITION OF delivery_log
    FOR VALUES FROM ('2026-03-01') TO ('2026-04-01');
CREATE TABLE delivery_log_p2026_04 PARTITION OF delivery_log
    FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');
CREATE TABLE delivery_log_p2026_05 PARTITION OF delivery_log
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE delivery_log_p2026_06 PARTITION OF delivery_log
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE delivery_log_p_future PARTITION OF delivery_log
    FOR VALUES FROM ('2026-07-01') TO (MAXVALUE);

CREATE INDEX idx_delivery_log_community_sub ON delivery_log (community_id, subscription_id);

-- ── Workflows ─────────────────────────────────────────────────────────────────
-- Conformance: "Workflows, runs, approvals, webhooks, schedules". Definition's
-- community fixed at create from req.community; runs/approvals inherit it.

CREATE TABLE workflows (
    community_id    UUID NOT NULL REFERENCES communities(id),
    id              UUID NOT NULL DEFAULT gen_random_uuid(),
    name            VARCHAR(255) NOT NULL,
    owner_pubkey    BYTEA NOT NULL,
    channel_id      UUID,
    definition      JSONB NOT NULL,
    definition_hash BYTEA NOT NULL,
    status          workflow_status NOT NULL DEFAULT 'active',
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_workflows_channel_active ON workflows (community_id, channel_id, status, enabled);
-- Scheduler scans enabled schedule workflows; community_id returned per row so
-- side effects run under the owning tenant's context (Lane0 contract §4a.5).
CREATE INDEX idx_workflows_enabled ON workflows (enabled, status) WHERE enabled;

-- ── Workflow runs ─────────────────────────────────────────────────────────────

CREATE TABLE workflow_runs (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    workflow_id         UUID NOT NULL,
    status              run_status NOT NULL DEFAULT 'pending',
    trigger_event_id    BYTEA,
    current_step        INT NOT NULL DEFAULT 0,
    execution_trace     JSONB NOT NULL DEFAULT '[]',
    trigger_context     JSONB,
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    error_message       TEXT,
    error_code          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs (community_id, workflow_id);
CREATE INDEX idx_workflow_runs_status ON workflow_runs (community_id, status);

-- ── Workflow approvals ────────────────────────────────────────────────────────
-- token-hash lookup scoped: approval token grants cannot act on another
-- community's same hash (conformance).

CREATE TABLE workflow_approvals (
    community_id    UUID NOT NULL REFERENCES communities(id),
    token           BYTEA NOT NULL,
    workflow_id     UUID NOT NULL,
    run_id          UUID NOT NULL,
    step_id         VARCHAR(64) NOT NULL,
    step_index      INT NOT NULL,
    approver_spec   TEXT NOT NULL,
    status          approval_status NOT NULL DEFAULT 'pending',
    approver_pubkey BYTEA,
    note            TEXT,
    granted_at      TIMESTAMPTZ,
    denied_at       TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, token),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_approvals_workflow ON workflow_approvals (community_id, workflow_id);
CREATE INDEX idx_workflow_approvals_run ON workflow_approvals (community_id, run_id);
CREATE INDEX idx_workflow_approvals_status ON workflow_approvals (community_id, status);

-- ── Scheduled workflow fires (cron claim) ─────────────────────────────────────
-- Plan §5: the at-most-once cron fire claim. UNIQUE (community_id, workflow_id,
-- scheduled_for) — only the pod that wins the claim insert creates the run.
-- Restart-safe (DB-durable). community is server provenance: the scheduler passes
-- workflow.community_id from list_all_enabled_workflows(), never a client input.
-- workflow_id is NOT globally unique under the (community_id, id) workflow key, so
-- the claim binds both community and id explicitly rather than resolving from id.
-- workflow_run_id links the won claim to the run it created (audit; NULL until the
-- post-insert attach, and stays NULL if run creation failed after a won claim).
-- The FK to workflow_runs uses NO ACTION (not SET NULL): community_id is shared
-- with the claim PK and is NOT NULL, so SET NULL is unimplementable here; a future
-- delete of a still-linked run is blocked rather than orphaning the at-most-once
-- claim row. workflow_runs are not pruned today, so this is a guardrail, not a path.

CREATE TABLE scheduled_workflow_fires (
    community_id    UUID NOT NULL REFERENCES communities(id),
    workflow_id     UUID NOT NULL,
    scheduled_for   TIMESTAMPTZ NOT NULL,
    claimed_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    workflow_run_id UUID,
    PRIMARY KEY (community_id, workflow_id, scheduled_for),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, workflow_run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE NO ACTION
);

-- The interval anchor reads MAX(scheduled_for) per workflow; the janitor prunes
-- by claimed_at globally (operator concern). See plan §5 retention coupling.
CREATE INDEX idx_scheduled_fires_claimed_at ON scheduled_workflow_fires (claimed_at);

-- ── API tokens ────────────────────────────────────────────────────────────────
-- Conformance: "API tokens and NIP-98 replay". token_hash uniqueness scoped to
-- (community_id, token_hash); channel claims reference channels in same community.

CREATE TABLE api_tokens (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    token_hash          BYTEA NOT NULL,
    owner_pubkey        BYTEA NOT NULL,
    name                VARCHAR(255) NOT NULL,
    scopes              JSONB NOT NULL,
    channel_ids         JSONB,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at          TIMESTAMPTZ,
    last_used_at        TIMESTAMPTZ,
    revoked_at          TIMESTAMPTZ,
    revoked_by          BYTEA,
    created_by_self_mint BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey),
    CONSTRAINT chk_api_tokens_hash_len CHECK (LENGTH(token_hash) = 32)
);

CREATE UNIQUE INDEX idx_api_tokens_hash ON api_tokens (community_id, token_hash);

-- ── Rate limit violations ─────────────────────────────────────────────────────
-- OPERATOR-GLOBAL: a deployment-health / abuse table, never tenant-observable.
-- Listed in the lint allowlist. Carries community_id as an attribution label
-- only (nullable, no uniqueness over it).

CREATE TABLE rate_limit_violations (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    community_id    UUID,
    pubkey          BYTEA,
    violation_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    limit_type      VARCHAR(64),
    limit_value     INT,
    actual_value    INT,
    action_taken    VARCHAR(64)
);

-- ── Thread metadata ───────────────────────────────────────────────────────────
-- Conformance: thread lookups filter by community before event matching.

CREATE TABLE thread_metadata (
    community_id            UUID NOT NULL REFERENCES communities(id),
    event_created_at        TIMESTAMPTZ NOT NULL,
    event_id                BYTEA NOT NULL,
    channel_id              UUID NOT NULL,
    parent_event_id         BYTEA,
    parent_event_created_at TIMESTAMPTZ,
    root_event_id           BYTEA,
    root_event_created_at   TIMESTAMPTZ,
    depth                   INT NOT NULL DEFAULT 0,
    reply_count             INT NOT NULL DEFAULT 0,
    descendant_count        INT NOT NULL DEFAULT 0,
    last_reply_at           TIMESTAMPTZ,
    broadcast               BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (community_id, event_created_at, event_id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_thread_metadata_parent ON thread_metadata (community_id, parent_event_id);
CREATE INDEX idx_thread_metadata_root ON thread_metadata (community_id, root_event_id);
CREATE INDEX idx_thread_metadata_channel_depth
    ON thread_metadata (community_id, channel_id, depth, event_created_at);
CREATE INDEX idx_thread_metadata_event_id ON thread_metadata (community_id, event_id);

-- ── Reactions ─────────────────────────────────────────────────────────────────
-- Conformance: reactions filter by community before event/pubkey matching.

CREATE TABLE reactions (
    community_id        UUID NOT NULL REFERENCES communities(id),
    event_created_at    TIMESTAMPTZ NOT NULL,
    event_id            BYTEA NOT NULL,
    pubkey              BYTEA NOT NULL,
    emoji               VARCHAR(66) NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    removed_at          TIMESTAMPTZ,
    reaction_event_id   BYTEA,
    PRIMARY KEY (community_id, event_created_at, event_id, pubkey, emoji)
);

CREATE INDEX idx_reactions_event ON reactions (community_id, event_id, event_created_at);
CREATE INDEX idx_reactions_pubkey ON reactions (community_id, pubkey);
-- A reaction's source event id is unique within a community.
CREATE UNIQUE INDEX idx_reactions_source_event ON reactions (community_id, reaction_event_id)
    WHERE reaction_event_id IS NOT NULL;

-- ── Pubkey allowlist ──────────────────────────────────────────────────────────
-- Conformance: "Relay membership, pubkey allowlist, archived identities".
-- PK becomes (community_id, pubkey).

CREATE TABLE pubkey_allowlist (
    community_id UUID NOT NULL REFERENCES communities(id),
    pubkey      BYTEA NOT NULL,
    added_by    BYTEA,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    note        TEXT,
    PRIMARY KEY (community_id, pubkey)
);

-- ── Relay members (NIP-43) ────────────────────────────────────────────────────
-- Conformance: membership gate, community-scoped. pubkey stored as hex TEXT
-- (unchanged wire form). PK (community_id, pubkey).

CREATE TABLE relay_members (
    community_id UUID NOT NULL REFERENCES communities(id),
    pubkey      TEXT NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    added_by    TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey)
);

CREATE INDEX idx_relay_members_role ON relay_members (community_id, role);

-- ── Join policy acceptances ──────────────────────────────────────────────────
-- Durable evidence of the policy version accepted when an invite claim grants
-- relay membership. The composite foreign key keeps evidence bound to a live
-- member in the same community and removes it with that membership.

CREATE TABLE join_policy_acceptances (
    community_id UUID NOT NULL,
    pubkey TEXT NOT NULL,
    policy_version TEXT NOT NULL CHECK (length(policy_version) = 64),
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey, policy_version),
    FOREIGN KEY (community_id, pubkey)
        REFERENCES relay_members (community_id, pubkey) ON DELETE CASCADE
);

-- ── Relay invites (use-limited invite links) ──────────────────────────────────
-- Conformance: durable invite records for atomic redemption, community-scoped.
-- Stores only SHA-256(code) as 32-byte BYTEA; never the reusable bearer code.
-- PK and UNIQUE both lead with community_id. max_uses NULL = unlimited.

CREATE TABLE relay_invites (
    community_id  UUID        NOT NULL REFERENCES communities(id),
    id           UUID        NOT NULL DEFAULT gen_random_uuid(),
    token_hash   BYTEA       NOT NULL CHECK (length(token_hash) = 32),
    role         TEXT        NOT NULL DEFAULT 'member' CHECK (role = 'member'),
    max_uses     INTEGER     CHECK (max_uses BETWEEN 1 AND 10000),
    use_count    INTEGER     NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    expires_at   TIMESTAMPTZ NOT NULL,
    created_by   TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, token_hash),
    CHECK (max_uses IS NULL OR use_count <= max_uses)
);

CREATE INDEX relay_invites_expires_at_idx ON relay_invites (expires_at);

-- ── Archived identities (NIP-IA) ──────────────────────────────────────────────
-- Conformance: archive cannot hide a key in another community. PK scoped.

CREATE TABLE archived_identities (
    community_id      UUID NOT NULL REFERENCES communities(id),
    pubkey            TEXT NOT NULL,
    consent_path      TEXT NOT NULL CHECK (consent_path IN ('self', 'owner', 'admin')),
    actor             TEXT NOT NULL,
    reason            TEXT,
    replaced_by       TEXT,
    request_event_id  TEXT NOT NULL,
    archived_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey)
);

-- ── Audit log ─────────────────────────────────────────────────────────────────
-- Conformance: "Audit log and observability". Per-community hash chain:
-- uniqueness (community_id, seq) and (community_id, hash). One chain per tenant.
-- (Lane Audit/Dawn builds the chain logic; Lane 0 fixes the scoped schema.)

CREATE TABLE audit_log (
    community_id    UUID NOT NULL REFERENCES communities(id),
    seq             BIGINT NOT NULL,
    hash            BYTEA NOT NULL,
    prev_hash       BYTEA,
    action          VARCHAR(64) NOT NULL,
    actor_pubkey    BYTEA,
    object_id       TEXT,
    detail          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, seq)
);

CREATE UNIQUE INDEX idx_audit_log_hash ON audit_log (community_id, hash);

-- ── NIP-56 reports (kind:1984 ingest) ─────────────────────────────────────────
-- One row per accepted report event. Reports are signals, never triggers:
-- nothing auto-actions on them (NIP-56). Reporter identity is visible to
-- moderators in the queue but never revealed to the reported author.

CREATE TABLE moderation_reports (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    -- The signed kind:1984 event id (stored for audit/idempotency).
    report_event_id     BYTEA NOT NULL CHECK (length(report_event_id) = 32),
    reporter_pubkey     BYTEA NOT NULL CHECK (length(reporter_pubkey) = 32),
    -- What was reported. Exactly one target class per row (CHECK-enforced below).
    target_kind         TEXT NOT NULL CHECK (target_kind IN ('event', 'pubkey', 'blob')),
    target_event_id     BYTEA CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    target_pubkey       BYTEA CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32),
    target_blob_sha256  BYTEA CHECK (target_blob_sha256 IS NULL OR length(target_blob_sha256) = 32),
    -- Channel inferred from an in-tenant target event row, when resolvable.
    channel_id          UUID,
    -- NIP-56 report type: illegal|nudity|malware|spam|impersonation|profanity|other.
    report_type         TEXT NOT NULL,
    -- Reporter's optional free-text context (mod-queue-only; never public).
    note                TEXT,
    status              TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open', 'processing', 'resolved', 'dismissed', 'escalated')),
    -- Non-null when status='processing': the relay_admin_actions row that claimed this report.
    active_action_id    UUID,
    resolved_by         BYTEA,
    resolved_at         TIMESTAMPTZ,
    -- moderation_actions row that resolved this report, if any.
    action_id           UUID,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    -- Exactly one target class per row: target_kind is authoritative and the
    -- matching column (only) is populated. Queue/action code never guesses.
    CHECK (
        (target_kind = 'event'  AND target_event_id IS NOT NULL AND target_pubkey IS NULL     AND target_blob_sha256 IS NULL) OR
        (target_kind = 'pubkey' AND target_event_id IS NULL     AND target_pubkey IS NOT NULL AND target_blob_sha256 IS NULL) OR
        (target_kind = 'blob'   AND target_event_id IS NULL     AND target_pubkey IS NULL     AND target_blob_sha256 IS NOT NULL)
    ),
    -- Same-community channel provenance (channels are soft-deleted, never
    -- hard-deleted, so this FK cannot dangle).
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

-- Queue reads: open reports, newest first, per community.
CREATE INDEX idx_moderation_reports_status
    ON moderation_reports (community_id, status, created_at DESC);
-- Group-by-target for triage aggregation.
CREATE INDEX idx_moderation_reports_target_event
    ON moderation_reports (community_id, target_event_id)
    WHERE target_event_id IS NOT NULL;
CREATE INDEX idx_moderation_reports_target_pubkey
    ON moderation_reports (community_id, target_pubkey)
    WHERE target_pubkey IS NOT NULL;
-- Idempotency: one row per report event per community.
CREATE UNIQUE INDEX idx_moderation_reports_event
    ON moderation_reports (community_id, report_event_id);

-- ── Bans + timeouts (one restriction row per member) ──────────────────────────
-- Ban = connection block, enforced at the NIP-42 auth seam
-- ("blocked: you are banned from this community") + join/ingest surfaces.
-- Timeout = write-block only ("restricted: you are timed out until <ts>").
-- A row may be ban-only, timeout-only, or both over its lifetime.

CREATE TABLE community_bans (
    community_id    UUID NOT NULL REFERENCES communities(id),
    pubkey          BYTEA NOT NULL CHECK (length(pubkey) = 32),
    banned          BOOLEAN NOT NULL DEFAULT false,
    -- NULL + banned=true ⇒ permanent.
    ban_expires_at  TIMESTAMPTZ,
    ban_reason      TEXT,
    -- Write-block until this timestamp; NULL or past ⇒ not timed out.
    muted_until     TIMESTAMPTZ,
    mute_reason     TEXT,
    -- Moderator who last modified this row.
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey)
);

-- ── Moderation audit ──────────────────────────────────────────────────────────
-- One row per accepted moderation action. Full detail (reporter identities,
-- private reasons, matched NIP-OA principal) stays mod/audit-only; the public
-- tombstone carries only action_id + reason_code + sanitized public_reason.

CREATE TABLE moderation_actions (
    community_id    UUID NOT NULL REFERENCES communities(id),
    id              UUID NOT NULL DEFAULT gen_random_uuid(),
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    action          TEXT NOT NULL CHECK (action IN (
                        'delete_message', 'kick', 'ban', 'unban',
                        'timeout', 'untimeout', 'dismiss_report', 'escalate',
                        'resolve:delete', 'resolve:kick', 'resolve:ban',
                        'resolve:timeout')),
    target_pubkey   BYTEA CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32),
    target_event_id BYTEA CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    channel_id      UUID,
    -- Machine-readable rule/reason code (e.g. "spam", "community_rule_3").
    reason_code     TEXT,
    -- Sanitized, safe for the public tombstone.
    public_reason   TEXT,
    -- Mod-only context; never leaves the audit surface.
    private_reason  TEXT,
    -- NIP-OA: which principal matched a ban ('self' | 'owner'); audit-only,
    -- the client never learns which.
    matched_principal TEXT CHECK (matched_principal IS NULL OR matched_principal IN ('self', 'owner')),
    -- Deployment authority type for HTTP-initiated actions.
    actor_authority   TEXT NOT NULL DEFAULT 'community'
                      CHECK (actor_authority IN ('community', 'relay_operator', 'relay_moderator')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_moderation_actions_created
    ON moderation_actions (community_id, created_at DESC);
CREATE INDEX idx_moderation_actions_target_pubkey
    ON moderation_actions (community_id, target_pubkey)
    WHERE target_pubkey IS NOT NULL;

-- Same-community resolution provenance: a report can only be resolved by an
-- action row in its own community. Added after moderation_actions exists.
ALTER TABLE moderation_reports
    ADD FOREIGN KEY (community_id, action_id)
    REFERENCES moderation_actions (community_id, id);

-- ── Lint allowlist registry ───────────────────────────────────────────────────
-- The explicit registry of tables that are deliberately operator-global (NOT
-- tenant-scoped). The migration-lint harness reads this: any table NOT listed
-- here MUST carry a NOT NULL community_id and lead its uniques with it. Making
-- the allowlist a DB table (not a hard-coded list in the linter) keeps the
-- registry next to the schema it governs and reviewable in one migration diff.

CREATE TABLE _operator_global_tables (
    table_name  TEXT PRIMARY KEY,
    reason      TEXT NOT NULL
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('communities',           'the tenant registry itself; id IS the community key'),
    ('rate_limit_violations', 'deployment abuse/health; never tenant-observable; community_id is an attribution label only'),
    ('_operator_global_tables', 'the registry table itself');

-- ── Additive tenant tables represented in migrations 0002/0007/0017 ──────────
-- Keep desired-state schema parity with the embedded SQLx migration path.
CREATE TABLE git_repo_names (
    community_id  UUID NOT NULL REFERENCES communities(id),
    repo_id       TEXT NOT NULL,
    owner_pubkey  TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, repo_id)
);
CREATE INDEX idx_git_repo_names_owner ON git_repo_names (community_id, owner_pubkey);

CREATE TABLE parameterized_event_watermarks (
    community_id  UUID NOT NULL REFERENCES communities(id),
    kind          INT NOT NULL,
    pubkey        BYTEA NOT NULL,
    d_tag         TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL,
    event_id      BYTEA NOT NULL,
    PRIMARY KEY (community_id, kind, pubkey, d_tag)
);
CREATE INDEX idx_event_mentions_community_event
    ON event_mentions (community_id, event_id);

CREATE TABLE product_feedback (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    community_id UUID REFERENCES communities(id) ON DELETE SET NULL,
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    submitter_pubkey BYTEA NOT NULL CHECK (length(submitter_pubkey) = 32),
    category TEXT CHECK (category IN ('bug', 'praise', 'needs-work')),
    body TEXT NOT NULL CHECK (length(btrim(body)) > 0),
    tags JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(tags) = 'array'),
    event_created_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Operator-managed lifecycle status.
    status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'reviewed', 'archived')),
    UNIQUE (event_id)
);
CREATE INDEX idx_product_feedback_received
    ON product_feedback (received_at DESC, id);
CREATE INDEX idx_product_feedback_community_received
    ON product_feedback (community_id, received_at DESC, id);
INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('product_feedback', 'deployment product inbox; community_id is provenance only');
-- NIP-PL effective lease state and durable wake outbox. Every key is led by
-- community_id: client-provided origin is confirmation only, never routing.
CREATE TABLE push_leases (
    community_id UUID NOT NULL REFERENCES communities(id),
    author BYTEA NOT NULL CHECK (length(author) = 32),
    installation_id TEXT NOT NULL CHECK (octet_length(installation_id) BETWEEN 1 AND 64),
    source_event_id BYTEA NOT NULL CHECK (length(source_event_id) = 32),
    source_created_at BIGINT NOT NULL,
    generation BIGINT NOT NULL CHECK (generation > 0),
    active BOOLEAN NOT NULL,
    endpoint_enabled BOOLEAN NOT NULL DEFAULT true,
    app_profile TEXT,
    endpoint_hash BYTEA CHECK (endpoint_hash IS NULL OR length(endpoint_hash) = 32),
    endpoint_grant TEXT,
    max_class TEXT CHECK (max_class IS NULL OR max_class IN ('silent','default','time_sensitive','urgent')),
    subscriptions JSONB,
    expires_at BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, author, installation_id),
    UNIQUE (community_id, source_event_id),
    CHECK ((active AND app_profile IS NOT NULL AND endpoint_hash IS NOT NULL AND endpoint_grant IS NOT NULL AND max_class IS NOT NULL AND subscriptions IS NOT NULL)
        OR (NOT active AND app_profile IS NULL AND endpoint_hash IS NULL AND endpoint_grant IS NULL AND max_class IS NULL AND subscriptions IS NULL))
);
CREATE UNIQUE INDEX push_leases_endpoint_unique
    ON push_leases (community_id, author, app_profile, endpoint_hash)
    WHERE active;
CREATE INDEX push_leases_expiry ON push_leases (community_id, expires_at) WHERE active;

CREATE TABLE push_wake_outbox (
    community_id UUID NOT NULL REFERENCES communities(id),
    id UUID NOT NULL DEFAULT gen_random_uuid(),
    author BYTEA NOT NULL CHECK (length(author) = 32),
    installation_id TEXT NOT NULL,
    lease_generation BIGINT NOT NULL CHECK (lease_generation > 0),
    endpoint_hash BYTEA NOT NULL CHECK (length(endpoint_hash) = 32),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    class TEXT NOT NULL CHECK (class IN ('silent','default','time_sensitive','urgent')),
    expires_at BIGINT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','sending','delivered','failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until TIMESTAMPTZ,
    claim_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, author, installation_id)
        REFERENCES push_leases (community_id, author, installation_id),
    UNIQUE (community_id, endpoint_hash, event_id)
);
CREATE INDEX push_wake_outbox_due
    ON push_wake_outbox (community_id, next_attempt_at) WHERE state = 'pending';
CREATE INDEX push_wake_outbox_recovery
    ON push_wake_outbox (community_id, lease_until) WHERE state = 'sending';
-- Durable event-to-push matching follower. The trigger runs in the event insert
-- transaction, so every accepted persistent event has a crash-safe match job and
-- rejected/rolled-back events never do. Processing is idempotent through the
-- push_wake_outbox endpoint/event unique key.
CREATE TABLE push_match_queue (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','matching')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until TIMESTAMPTZ,
    claim_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, event_id)
);
CREATE INDEX push_match_queue_due
    ON push_match_queue (next_attempt_at, created_at) WHERE state = 'pending';
CREATE INDEX push_match_queue_recovery
    ON push_match_queue (lease_until) WHERE state = 'matching';

-- T1b push gate (keep in sync with migrations/0023). Enqueue only when the
-- community has an active, endpoint-enabled, unexpired lease; the shared
-- advisory lock pairs with the exclusive lock taken by lease activations
-- (crates/buzz-db/src/push.rs) to close the lost-wake race.
CREATE FUNCTION enqueue_push_match_job() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- Keep this allowlist identical to the relay's validated NIP-PL descriptor.
    -- Centralizing it on the events table covers every durable producer,
    -- including internal paths that bypass live dispatch.
    IF NEW.kind IN (9, 40002, 45001, 45003) THEN
        PERFORM pg_advisory_xact_lock_shared(
            hashtextextended('buzz_push_gate:' || NEW.community_id::text, 0));
        IF EXISTS (
            SELECT 1 FROM push_leases
            WHERE community_id = NEW.community_id
              AND active
              AND endpoint_enabled
              AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        ) THEN
            INSERT INTO push_match_queue (community_id, event_id)
            VALUES (NEW.community_id, NEW.id)
            ON CONFLICT DO NOTHING;
        END IF;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER events_enqueue_push_match
AFTER INSERT ON events
FOR EACH ROW EXECUTE FUNCTION enqueue_push_match_job();

-- Channel TTL refresh (keep in sync with migrations/0024). Runs deferred, in
-- the transaction that makes a channel-scoped event durable, so a TTL
-- transition committed while ingest was in flight is never missed. The
-- per-channel advisory lock is SHARED here — permanent-channel commits admit
-- each other — and taken EXCLUSIVE by TTL transitions (update_channel in
-- crates/buzz-db/src/channel.rs), which forces the same total order the
-- 0022 row lock provided without serializing the hot path.
CREATE FUNCTION refresh_channel_ttl_after_event_insert() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    channel_ttl INTEGER;
BEGIN
    -- Kind 9007 creates the channel and initializes its deadline itself.
    IF NEW.channel_id IS NOT NULL AND NEW.kind <> 9007 THEN
        BEGIN
            PERFORM pg_advisory_xact_lock_shared(hashtextextended(
                'buzz_channel_ttl:' || NEW.community_id::text || ':' || NEW.channel_id::text, 0));

            SELECT ttl_seconds INTO channel_ttl
            FROM channels
            WHERE community_id = NEW.community_id AND id = NEW.channel_id;

            IF channel_ttl IS NOT NULL THEN
                UPDATE channels
                SET ttl_deadline = clock_timestamp() + make_interval(secs => ttl_seconds)
                WHERE community_id = NEW.community_id
                  AND id = NEW.channel_id
                  AND ttl_seconds IS NOT NULL
                  AND archived_at IS NULL
                  AND deleted_at IS NULL;
            END IF;
        EXCEPTION WHEN OTHERS THEN
            -- Preserve the existing best-effort contract: a TTL refresh failure
            -- must not reject an otherwise valid durable event.
            RAISE WARNING 'channel TTL refresh failed for community %, channel %: %',
                NEW.community_id, NEW.channel_id, SQLERRM;
        END;
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
AFTER INSERT ON events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

-- Channel roster snapshot fence (keep in sync with migrations/0032).
-- Prevent mixed-version relay pods from publishing a stale NIP-29 member
-- snapshot after a newer canonical roster has been committed.
--
-- Old binaries already serialize kind 39002 replacement on the replacement
-- advisory key. This trigger adds the channel-membership key at INSERT time,
-- after that canonical key, and validates every p tag against the current
-- active membership set and roles. New binaries take both keys in the same
-- order before capture and replacement. Thus old and new writers remain
-- compatible during a rolling deploy.
CREATE OR REPLACE FUNCTION guard_channel_roster_snapshot()
RETURNS TRIGGER AS $$
DECLARE
    canonical_members TEXT[];
    snapshot_members TEXT[];
BEGIN
    IF NEW.kind <> 39002 OR NEW.channel_id IS NULL THEN
        RETURN NEW;
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended(
        'buzz_channel_membership:' || NEW.community_id::text || ':' || NEW.channel_id::text,
        0
    ));

    SELECT COALESCE(
               array_agg(encode(cm.pubkey, 'hex') || ':' || cm.role::text ORDER BY cm.pubkey),
               ARRAY[]::TEXT[]
           )
      INTO canonical_members
      FROM channel_members cm
     WHERE cm.community_id = NEW.community_id
       AND cm.channel_id = NEW.channel_id
       AND cm.removed_at IS NULL;

    -- A roster is canonical only when every p tag uses the emitted four-field
    -- shape, contains a 32-byte hex pubkey and valid authoritative role, has no
    -- duplicate members, and exactly matches the active membership rows.
    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.tags) AS roster_tag(tag_json)
         WHERE roster_tag.tag_json->>0 = 'p'
           AND (
               jsonb_array_length(roster_tag.tag_json) <> 4
               OR COALESCE(roster_tag.tag_json->>1, '') !~ '^[0-9a-fA-F]{64}$'
               OR roster_tag.tag_json->>2 <> ''
               OR COALESCE(roster_tag.tag_json->>3, '') NOT IN ('owner', 'admin', 'bot', 'member', 'guest')
           )
    ) THEN
        RAISE EXCEPTION 'kind 39002 roster contains an invalid p tag'
            USING ERRCODE = '23514';
    END IF;

    SELECT COALESCE(
               array_agg(
                   lower((roster_tag.tag_json->>1)) || ':' || (roster_tag.tag_json->>3)
                   ORDER BY decode((roster_tag.tag_json->>1), 'hex')
               ),
               ARRAY[]::TEXT[]
           )
      INTO snapshot_members
      FROM jsonb_array_elements(NEW.tags) AS roster_tag(tag_json)
     WHERE roster_tag.tag_json->>0 = 'p';

    IF snapshot_members IS DISTINCT FROM canonical_members THEN
        RAISE EXCEPTION 'kind 39002 roster does not match canonical channel membership'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_events_guard_channel_roster_snapshot ON events;
CREATE TRIGGER trg_events_guard_channel_roster_snapshot
    BEFORE INSERT ON events
    FOR EACH ROW EXECUTE FUNCTION guard_channel_roster_snapshot();


-- Replica-fence floor guard (keep in sync with migrations/0021). A deferred
-- constraint trigger re-checks, inside COMMIT processing, that channel-bearing
-- event rows are no older than `buzz.created_at_floor` seconds before commit
-- time (clock_timestamp(), NOT the transaction-frozen now()). This turns the
-- relay's ingest-time created_at envelope into a commit-time storage
-- invariant, which is what lets keyset-cursor pages below the replica fence
-- be served by a read replica without holes. Enforcement is armed per session
-- via the GUC (set by the relay's writer pool on connect); sessions without
-- the GUC (pg_restore, manual backfills) bypass it and must hold the replica
-- fence closed for their duration. The only structural exemption is
-- channel_id IS NULL: those rows never appear in keyset-paged windows.
CREATE FUNCTION events_created_at_floor_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    floor_secs numeric := nullif(current_setting('buzz.created_at_floor', true), '')::numeric;
BEGIN
    IF floor_secs IS NOT NULL
       AND floor_secs > 0
       AND NEW.channel_id IS NOT NULL
       AND NEW.created_at < clock_timestamp() - make_interval(secs => floor_secs)
    THEN
        RAISE EXCEPTION
            'events.created_at % is more than % s before commit time %; below the replica-fence floor',
            NEW.created_at, floor_secs, clock_timestamp()
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

-- INSERT OR UPDATE OF: an UPDATE can move a previously exempt row into the
-- guarded set (channel_id NULL -> NOT NULL) or move a channel row's
-- created_at below the fence, so both mutation paths re-run the guard on the
-- NEW row. A created_at rewrite that crosses partition bounds runs as
-- DELETE + INSERT and hits the cloned AFTER INSERT guard on the destination
-- partition; an in-partition rewrite fires the UPDATE OF arm.
CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

-- Durable, deployment-global authority for the public NIP-PL push gateway.
-- This state is intentionally outside relay community tenancy: installations
-- delegate to relay signing keys and may authorize multiple relay deployments.
CREATE TABLE push_gateway_challenges (
    id UUID PRIMARY KEY,
    challenge_hash BYTEA NOT NULL CHECK (length(challenge_hash) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX push_gateway_challenges_expiry ON push_gateway_challenges (expires_at);

CREATE TABLE push_gateway_installations (
    id UUID PRIMARY KEY,
    app_attest_key_id BYTEA NOT NULL UNIQUE CHECK (octet_length(app_attest_key_id) BETWEEN 1 AND 128),
    app_attest_public_key BYTEA NOT NULL CHECK (octet_length(app_attest_public_key) BETWEEN 33 AND 256),
    assertion_counter BIGINT NOT NULL CHECK (assertion_counter BETWEEN 0 AND 4294967295),
    app_profile TEXT NOT NULL CHECK (app_profile IN ('buzz-ios-production','buzz-ios-sandbox')),
    token_ciphertext BYTEA NOT NULL CHECK (octet_length(token_ciphertext) BETWEEN 1 AND 2048),
    token_fingerprint BYTEA NOT NULL CHECK (length(token_fingerprint) = 32),
    endpoint_epoch BIGINT NOT NULL CHECK (endpoint_epoch > 0),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (app_profile, token_fingerprint)
);
CREATE INDEX push_gateway_installations_expiry ON push_gateway_installations (expires_at) WHERE revoked_at IS NULL;

CREATE TABLE push_gateway_delegations (
    id UUID PRIMARY KEY,
    installation_id UUID NOT NULL REFERENCES push_gateway_installations(id),
    relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
    endpoint_epoch BIGINT NOT NULL CHECK (endpoint_epoch > 0),
    generation BIGINT NOT NULL CHECK (generation > 0),
    not_before TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (installation_id, relay_pubkey),
    CHECK (not_before < expires_at)
);
CREATE INDEX push_gateway_delegations_expiry ON push_gateway_delegations (expires_at) WHERE revoked_at IS NULL;

CREATE TABLE push_gateway_endpoint_quotas (
    token_fingerprint BYTEA PRIMARY KEY CHECK (length(token_fingerprint) = 32),
    window_started_at TIMESTAMPTZ NOT NULL,
    admitted BIGINT NOT NULL CHECK (admitted >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX push_gateway_endpoint_quotas_updated ON push_gateway_endpoint_quotas (updated_at);

CREATE TABLE push_gateway_delivery_auth_replays (
    relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
    auth_event_id BYTEA NOT NULL CHECK (length(auth_event_id) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (relay_pubkey, auth_event_id)
);
CREATE INDEX push_gateway_delivery_auth_replays_expiry ON push_gateway_delivery_auth_replays (expires_at);

CREATE TABLE push_gateway_delivery_request_replays (
    relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
    request_id UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (relay_pubkey, request_id)
);
CREATE INDEX push_gateway_delivery_request_replays_expiry ON push_gateway_delivery_request_replays (expires_at);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('push_gateway_challenges', 'public gateway one-time challenges span relay communities'),
    ('push_gateway_installations', 'public gateway installation authority spans relay communities'),
    ('push_gateway_delegations', 'public gateway relay delegations span relay communities'),
    ('push_gateway_endpoint_quotas', 'public gateway endpoint abuse ceilings span relay communities'),
    ('push_gateway_delivery_auth_replays', 'public gateway signed-event replay admission spans relay communities'),
    ('push_gateway_delivery_request_replays', 'public gateway stable request-id admission spans relay communities');

-- ── Replica heartbeat (read-replica freshness fence) ─────────────────────────
-- Portable read-side freshness observation for the replica fence (see
-- crates/buzz-db/src/replica_fence.rs and migrations/0026). Exactly one row;
-- the single-row token UPDATE is the serialization point that makes tokens
-- globally commit-ordered across relay pods. `epoch` detects token resets
-- (restore/re-seed) so a stale retained token can never masquerade as fresh
-- coverage. Deployment-global by design: describes replication topology,
-- never tenant data.

CREATE TABLE replica_heartbeat (
    id    smallint PRIMARY KEY CHECK (id = 1),
    epoch uuid     NOT NULL DEFAULT gen_random_uuid(),
    token bigint   NOT NULL DEFAULT 0
) WITH (
    vacuum_truncate = false
);

INSERT INTO replica_heartbeat (id) VALUES (1);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('replica_heartbeat', 'single-row replication freshness token; describes deployment topology, never tenant data');

-- ── Whole-community deletion control plane (migration 0029) ─────────────────
CREATE TABLE community_deletion_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    community_id UUID NOT NULL REFERENCES communities(id),
    community_host TEXT NOT NULL,
    stage TEXT NOT NULL DEFAULT 'submitted' CHECK (stage IN (
        'submitted', 'inventoried', 'approved', 'fenced', 'drained',
        'bindings_removed', 'postgres_purged', 'cache_purged',
        'logically_verified', 'retention_pending', 'aborted'
    )),
    requested_by TEXT NOT NULL,
    reason TEXT,
    schema_manifest JSONB,
    storage_manifest JSONB,
    destructive_storage_manifest JSONB,
    destructive_storage_frozen_at TIMESTAMPTZ,
    inventory_manifest JSONB,
    inventory_digest BYTEA CHECK (inventory_digest IS NULL OR length(inventory_digest) = 32),
    inventory_frozen_at TIMESTAMPTZ,
    fence_generation BIGINT CHECK (fence_generation IS NULL OR fence_generation > 0),
    lease_owner TEXT,
    lease_generation BIGINT NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    retry_stage TEXT CHECK (retry_stage IS NULL OR retry_stage IN (
        'approved', 'fenced', 'drained', 'bindings_removed',
        'postgres_purged', 'cache_purged', 'logically_verified'
    )),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    last_error_at TIMESTAMPTZ,
    blocked_at TIMESTAMPTZ,
    blocked_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    pre_quiesce_archived_at TIMESTAMPTZ,
    quiescing_started_at TIMESTAMPTZ,
    aborted_by TEXT,
    abort_reason TEXT,
    aborted_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    CHECK ((blocked_at IS NULL) = (blocked_reason IS NULL)),
    CHECK ((stage = 'aborted') = (aborted_at IS NOT NULL)),
    CHECK ((aborted_at IS NULL) = (aborted_by IS NULL)),
    CHECK ((aborted_at IS NULL) = (abort_reason IS NULL)),
    CHECK ((inventory_frozen_at IS NULL) = (inventory_digest IS NULL)),
    UNIQUE (id, community_id, inventory_digest)
);
CREATE UNIQUE INDEX community_deletion_requests_active_community
    ON community_deletion_requests (community_id)
    WHERE stage <> 'aborted';
CREATE INDEX community_deletion_requests_runnable
    ON community_deletion_requests (next_attempt_at, created_at)
    WHERE blocked_at IS NULL
      AND stage IN ('approved', 'fenced', 'drained', 'bindings_removed',
                    'postgres_purged', 'cache_purged', 'logically_verified');
CREATE INDEX community_deletion_requests_lease
    ON community_deletion_requests (lease_until) WHERE lease_owner IS NOT NULL;

CREATE TABLE community_deletion_approvals (
    request_id UUID PRIMARY KEY,
    community_id UUID NOT NULL,
    inventory_digest BYTEA NOT NULL CHECK (length(inventory_digest) = 32),
    approved_by TEXT NOT NULL,
    note TEXT,
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (request_id, community_id, inventory_digest)
        REFERENCES community_deletion_requests(id, community_id, inventory_digest)
        ON DELETE RESTRICT
);

CREATE FUNCTION prevent_community_deletion_request_retargeting()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.community_host IS DISTINCT FROM OLD.community_host
    THEN
        RAISE EXCEPTION 'community deletion target identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.inventory_frozen_at IS NOT NULL AND (
        NEW.schema_manifest IS DISTINCT FROM OLD.schema_manifest
        OR NEW.storage_manifest IS DISTINCT FROM OLD.storage_manifest
        OR NEW.inventory_manifest IS DISTINCT FROM OLD.inventory_manifest
        OR NEW.inventory_digest IS DISTINCT FROM OLD.inventory_digest
        OR NEW.inventory_frozen_at IS DISTINCT FROM OLD.inventory_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen community deletion inventory is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.destructive_storage_frozen_at IS NOT NULL AND (
        NEW.destructive_storage_manifest IS DISTINCT FROM OLD.destructive_storage_manifest
        OR NEW.destructive_storage_frozen_at IS DISTINCT FROM OLD.destructive_storage_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen destructive storage manifest is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER community_deletion_request_retargeting_guard
BEFORE UPDATE ON community_deletion_requests
FOR EACH ROW
EXECUTE FUNCTION prevent_community_deletion_request_retargeting();

CREATE FUNCTION prevent_community_deletion_approval_removal()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'community deletion approval evidence is immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END;
$$;

CREATE TRIGGER community_deletion_approval_removal_guard
BEFORE UPDATE OR DELETE ON community_deletion_approvals
FOR EACH ROW
EXECUTE FUNCTION prevent_community_deletion_approval_removal();

CREATE TABLE community_deletion_checkpoints (
    request_id UUID NOT NULL REFERENCES community_deletion_requests(id) ON DELETE RESTRICT,
    sequence BIGINT GENERATED ALWAYS AS IDENTITY,
    stage TEXT NOT NULL,
    unit_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('started', 'completed', 'failed')),
    lease_generation BIGINT NOT NULL CHECK (lease_generation > 0),
    attempts INTEGER NOT NULL DEFAULT 1 CHECK (attempts > 0),
    detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    error TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (request_id, sequence),
    UNIQUE (request_id, stage, unit_key),
    CHECK ((status = 'completed') = (completed_at IS NOT NULL)),
    CHECK ((status = 'failed') = (error IS NOT NULL))
);

-- Frozen destructive key list, chunked out of the request row so a large
-- tenant (100k-1M objects) never materializes as one multi-hundred-MB JSONB
-- value. Rows are written once in the fenced stage, stamped `deleted_at` as
-- the executor confirms each chunk removed, and dropped at logical
-- verification. The request row keeps only per-prefix count/bytes/digest
-- summaries; the chunk stream must hash to those frozen digests.
CREATE TABLE community_deletion_manifest_keys (
    request_id UUID NOT NULL REFERENCES community_deletion_requests(id) ON DELETE CASCADE,
    chunk_no BIGINT NOT NULL CHECK (chunk_no >= 0),
    prefix TEXT NOT NULL,
    keys JSONB NOT NULL,
    deleted_at TIMESTAMPTZ,
    PRIMARY KEY (request_id, chunk_no)
);

-- Chunk content is immutable once written; the only permitted update is the
-- one-way deleted_at stamp. New chunks are permitted only while the request is
-- fenced and its destructive manifest remains unfrozen. Removal is permitted
-- only while the destructive manifest has not yet frozen (a retried partial
-- freeze rewrites its chunks) or once the request has passed logical
-- verification (terminal cleanup).
CREATE FUNCTION protect_community_deletion_manifest_keys()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    frozen_at TIMESTAMPTZ;
    request_stage TEXT;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.request_id IS DISTINCT FROM OLD.request_id
            OR NEW.chunk_no IS DISTINCT FROM OLD.chunk_no
            OR NEW.prefix IS DISTINCT FROM OLD.prefix
            OR NEW.keys IS DISTINCT FROM OLD.keys
            OR OLD.deleted_at IS NOT NULL
        THEN
            RAISE EXCEPTION 'community deletion manifest key chunks are immutable'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        RETURN NEW;
    END IF;
    SELECT destructive_storage_frozen_at, stage
      INTO frozen_at, request_stage
      FROM community_deletion_requests
     WHERE id = CASE WHEN TG_OP = 'INSERT' THEN NEW.request_id ELSE OLD.request_id END
     FOR UPDATE;
    IF TG_OP = 'INSERT' THEN
        IF FOUND AND frozen_at IS NULL AND request_stage = 'fenced' THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'community deletion manifest key chunks require an unfrozen fenced request'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NOT FOUND
        OR frozen_at IS NULL
        OR request_stage IN ('logically_verified', 'retention_pending')
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'community deletion manifest key chunks cannot be removed mid-execution'
        USING ERRCODE = 'integrity_constraint_violation';
END;
$$;

CREATE TRIGGER community_deletion_manifest_keys_guard
BEFORE INSERT OR UPDATE OR DELETE ON community_deletion_manifest_keys
FOR EACH ROW
EXECUTE FUNCTION protect_community_deletion_manifest_keys();

-- Fleet-wide object-store taxonomy sweep evidence. This is an independent
-- observability record: community deletion inventories only the target's owned
-- prefixes and does not gate submission or execution on sweep state.
CREATE TABLE storage_taxonomy_sweeps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    listed_objects BIGINT NOT NULL CHECK (listed_objects >= 0),
    unknown_object_count BIGINT NOT NULL CHECK (unknown_object_count >= 0),
    unknown_key_sample JSONB NOT NULL DEFAULT '[]'::jsonb,
    object_cap BIGINT NOT NULL CHECK (object_cap > 0),
    CHECK (completed_at >= started_at)
);
CREATE INDEX storage_taxonomy_sweeps_latest
    ON storage_taxonomy_sweeps (completed_at DESC);

CREATE TABLE community_serving_write_leases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    community_id UUID NOT NULL REFERENCES communities(id),
    operation TEXT NOT NULL,
    owner TEXT NOT NULL,
    generation BIGINT NOT NULL DEFAULT 1 CHECK (generation > 0),
    -- Community fence generation observed when this lease was acquired.
    fence_generation BIGINT NOT NULL CHECK (fence_generation >= 0),
    lease_until TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX community_serving_write_leases_active
    ON community_serving_write_leases (community_id, lease_until);

CREATE TABLE community_deletion_executor_heartbeats (
    executor_id TEXT PRIMARY KEY,
    mode TEXT NOT NULL CHECK (mode IN ('run', 'drain', 'worker')),
    request_id UUID REFERENCES community_deletion_requests(id) ON DELETE SET NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    draining BOOLEAN NOT NULL DEFAULT false,
    stopped_at TIMESTAMPTZ
);
INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('community_deletion_requests', 'deployment deletion lifecycle and frozen inventory'),
    ('community_deletion_approvals', 'deployment operator destructive approvals'),
    ('community_deletion_checkpoints', 'deployment deletion executor checkpoints and failures'),
    ('community_deletion_manifest_keys', 'deployment deletion frozen destructive key chunks'),
    ('storage_taxonomy_sweeps', 'deployment object-store taxonomy sweep evidence'),
    ('community_serving_write_leases', 'deployment serving side-effect leases drained by deletion'),
    ('community_deletion_executor_heartbeats', 'deployment deletion worker liveness');

CREATE FUNCTION community_deletion_lock_key(target UUID) RETURNS BIGINT
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT hashtextextended('buzz-community-deletion:' || target::text, 0)
$$;
-- Keep the deletion control plane writable while its target tenant is fenced.
-- This predicate is the single SQL source of truth used by attachment and live
-- catalog validation.
CREATE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations',
        'authorization_operation_receipts', 'identity_enrollment_policies',
        'identity_bindings', 'identity_lifecycle_history',
        'identity_lifecycle_selectors',
        'authorization_invalidation_domains', 'authorization_invalidation_floors',
        'authorization_authority_epochs', 'protected_object_authority',
        'authorization_event_capacity', 'authorization_events',
        'authorization_authentication_denial_attempts',
        'authorization_operation_version_delta_manifests',
        'authorization_operation_version_deltas', 'authorization_admission_results'
    ]::TEXT[])
$$;

-- Fleet-wide writers filter candidates through this VOLATILE predicate in
-- the mutating statement so fenced tenants are skipped before row triggers run.
CREATE FUNCTION community_write_allowed(target UUID) RETURNS BOOLEAN
LANGUAGE plpgsql VOLATILE AS $$
DECLARE
    lifecycle TEXT;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'community writes require READ COMMITTED isolation'
            USING ERRCODE = 'invalid_transaction_state';
    END IF;

    IF target IS NULL THEN
        RETURN true;
    END IF;

    PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(target));
    SELECT deletion_state
      INTO lifecycle
      FROM communities
     WHERE id = target;
    RETURN FOUND AND lifecycle = 'active';
END
$$;

CREATE FUNCTION assert_community_write_allowed(target UUID) RETURNS VOID
LANGUAGE plpgsql AS $$
DECLARE
    lifecycle TEXT;
    generation BIGINT;
    executor_community TEXT;
    executor_generation TEXT;
    serving_community TEXT;
    serving_lease_id TEXT;
    serving_owner TEXT;
    serving_generation TEXT;
    serving_fence_generation TEXT;
    serving_lease_valid BOOLEAN := false;
BEGIN
    -- The fence proof requires a fresh statement snapshot after lock grant;
    -- pinned RR/Serializable snapshots can retain pre-fence authorization.
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'community writes require READ COMMITTED isolation'
            USING ERRCODE = 'invalid_transaction_state';
    END IF;

    -- Nullable operator-attribution rows without a tenant are unrelated.
    IF target IS NULL THEN
        RETURN;
    END IF;

    PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(target));
    SELECT deletion_state, deletion_fence_generation
      INTO lifecycle, generation
      FROM communities
     WHERE id = target;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'community write rejected: community % is missing', target
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;

    -- Authorization is evaluated independently for every community checked.
    executor_community := current_setting('buzz.deletion_executor_community', true);
    executor_generation := current_setting('buzz.deletion_fence_generation', true);
    IF executor_community = target::TEXT
       AND executor_generation ~ '^[0-9]+$'
       AND executor_generation::BIGINT = generation THEN
        RETURN;
    END IF;

    -- A serving mutation admitted before quiescing may finish only while its
    -- exact durable lease remains current and bound to this fence generation.
    serving_community := current_setting('buzz.serving_write_community', true);
    serving_lease_id := current_setting('buzz.serving_write_lease_id', true);
    serving_owner := current_setting('buzz.serving_write_owner', true);
    serving_generation := current_setting('buzz.serving_write_generation', true);
    serving_fence_generation := current_setting('buzz.serving_write_fence_generation', true);
    IF lifecycle IN ('active', 'quiescing')
       AND serving_community = target::TEXT
       AND serving_lease_id ~ '^[0-9a-fA-F-]{36}$'
       AND serving_generation ~ '^[0-9]+$'
       AND serving_fence_generation ~ '^[0-9]+$'
       AND serving_fence_generation::BIGINT = generation THEN
        SELECT EXISTS(
            SELECT 1 FROM community_serving_write_leases lease
             WHERE lease.id = serving_lease_id::UUID
               AND lease.community_id = target
               AND lease.owner = serving_owner
               AND lease.generation = serving_generation::BIGINT
               AND lease.fence_generation = serving_fence_generation::BIGINT
               AND lease.lease_until >= now()
        ) INTO serving_lease_valid;
        IF serving_lease_valid THEN
            RETURN;
        END IF;
    END IF;

    IF lifecycle <> 'active' THEN
        RAISE EXCEPTION 'community write fenced: community % generation %', target, generation
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$$;

CREATE FUNCTION enforce_community_write_fence() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        PERFORM assert_community_write_allowed(NEW.community_id);
    ELSIF TG_OP = 'DELETE' THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
    ELSIF OLD.community_id IS NOT DISTINCT FROM NEW.community_id THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
    ELSIF OLD.community_id IS NULL THEN
        PERFORM assert_community_write_allowed(NEW.community_id);
    ELSIF NEW.community_id IS NULL THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
    ELSIF OLD.community_id < NEW.community_id THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
        PERFORM assert_community_write_allowed(NEW.community_id);
    ELSE
        PERFORM assert_community_write_allowed(NEW.community_id);
        PERFORM assert_community_write_allowed(OLD.community_id);
    END IF;

    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;

CREATE FUNCTION enforce_community_tombstone() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    executor_community TEXT := current_setting('buzz.deletion_executor_community', true);
    executor_generation TEXT := current_setting('buzz.deletion_fence_generation', true);
    expected_generation BIGINT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.deletion_state <> 'active' OR OLD.deleted_at IS NOT NULL THEN
            RAISE EXCEPTION 'community tombstones are permanent'
                USING ERRCODE = 'object_not_in_prerequisite_state';
        END IF;
        RETURN OLD;
    END IF;
    expected_generation := CASE WHEN NEW.deletion_fence_generation > OLD.deletion_fence_generation
        THEN NEW.deletion_fence_generation ELSE OLD.deletion_fence_generation END;
    IF executor_community = OLD.id::text AND executor_generation ~ '^[0-9]+$'
       AND executor_generation::BIGINT = expected_generation THEN RETURN NEW; END IF;
    IF OLD.deletion_state <> 'active' OR NEW.deletion_state <> OLD.deletion_state
       OR NEW.deletion_fence_generation <> OLD.deletion_fence_generation
       OR NEW.deleted_at IS DISTINCT FROM OLD.deleted_at THEN
        RAISE EXCEPTION 'community tombstone mutation rejected: community % generation %',
            OLD.id, OLD.deletion_fence_generation
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER communities_deletion_tombstone BEFORE UPDATE OR DELETE ON communities
FOR EACH ROW EXECUTE FUNCTION enforce_community_tombstone();
-- Attach the universal fence to one community-scoped relation. Future
-- migrations must invoke this helper explicitly after CREATE/ALTER introduces
-- community_id; the migration lint enforces that contract.
CREATE FUNCTION attach_community_write_fence(target REGCLASS) RETURNS VOID
LANGUAGE plpgsql AS $$
DECLARE
    relation_name NAME;
BEGIN
    SELECT c.relname
      INTO relation_name
      FROM pg_class c
      JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE c.oid = target
       AND n.nspname = current_schema()
       AND c.relkind IN ('r', 'p')
       AND NOT c.relispartition;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'community write fence target % is not a table in the current schema', target
            USING ERRCODE = 'wrong_object_type';
    END IF;
    IF community_write_fence_excluded_table(relation_name) THEN
        RETURN;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_attribute
         WHERE attrelid = target AND attname = 'community_id' AND NOT attisdropped
    ) THEN
        RAISE EXCEPTION 'community write fence target % has no community_id', target
            USING ERRCODE = 'undefined_column';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
         WHERE tgrelid = target
           AND tgname = 'community_write_fence_' || relation_name
           AND NOT tgisinternal
    ) THEN
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE INSERT OR UPDATE OR DELETE ON %s '
            'FOR EACH ROW EXECUTE FUNCTION enforce_community_write_fence()',
            'community_write_fence_' || relation_name,
            target
        );
    END IF;
END
$$;

-- Attach the universal fence to every existing table carrying community_id,
-- including deployment-private sidecars whose community_id is provenance.
DO $$
DECLARE
    target REGCLASS;
BEGIN
    FOR target IN
        SELECT c.oid::REGCLASS
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          JOIN pg_attribute a ON a.attrelid = c.oid
         WHERE n.nspname = current_schema()
           AND c.relkind IN ('r', 'p')
           AND NOT c.relispartition
           AND a.attname = 'community_id'
           AND NOT a.attisdropped
           AND NOT community_write_fence_excluded_table(c.relname)
         ORDER BY c.oid::REGCLASS::TEXT
    LOOP
        PERFORM attach_community_write_fence(target);
    END LOOP;
END
$$;

-- Desired-state schema application does not replay migration history, so keep
-- these explicit calls as first-class catalog declarations. They also make the
-- fence contract visible to migration linting instead of hiding it only in the
-- dynamic bootstrap loop above.
SELECT attach_community_write_fence('api_tokens');
SELECT attach_community_write_fence('archived_identities');
SELECT attach_community_write_fence('audit_log');
SELECT attach_community_write_fence('channel_members');
SELECT attach_community_write_fence('channels');
SELECT attach_community_write_fence('community_bans');
SELECT attach_community_write_fence('delivery_log');
SELECT attach_community_write_fence('event_mentions');
SELECT attach_community_write_fence('events');
SELECT attach_community_write_fence('git_repo_names');
SELECT attach_community_write_fence('join_policy_acceptances');
SELECT attach_community_write_fence('moderation_actions');
SELECT attach_community_write_fence('moderation_reports');
SELECT attach_community_write_fence('parameterized_event_watermarks');
SELECT attach_community_write_fence('pubkey_allowlist');
SELECT attach_community_write_fence('push_leases');
SELECT attach_community_write_fence('push_match_queue');
SELECT attach_community_write_fence('push_wake_outbox');
SELECT attach_community_write_fence('reactions');
SELECT attach_community_write_fence('relay_invites');
SELECT attach_community_write_fence('relay_members');
SELECT attach_community_write_fence('scheduled_workflow_fires');
SELECT attach_community_write_fence('subscriptions');
SELECT attach_community_write_fence('thread_metadata');
SELECT attach_community_write_fence('users');
SELECT attach_community_write_fence('workflow_approvals');
SELECT attach_community_write_fence('workflow_runs');
SELECT attach_community_write_fence('workflows');

-- ── Relay operator/moderator roster ──────────────────────────────────────────
-- Deployment-level principals staffed via the admin API. Config-backed operators
-- (RELAY_OPERATOR_PUBKEYS, RELAY_OWNER_PUBKEY owner-fallback) are NOT seeded here;
-- they are authoritative in config and outrank any DB row.

CREATE TABLE relay_operators (
    pubkey      BYTEA NOT NULL PRIMARY KEY CHECK (length(pubkey) = 32),
    role        TEXT NOT NULL CHECK (role IN ('operator', 'moderator')),
    added_by    BYTEA NOT NULL CHECK (length(added_by) = 32),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('relay_operators', 'deployment-global operator/moderator roster; no community_id intentionally');

-- ── Relay admin actions (HTTP enforcement state machine) ──────────────────────
-- One row per HTTP report-resolution enforcement action. Tracks the durable
-- state machine from claim → enforcing → succeeded|failed|cancelled.

CREATE TABLE relay_admin_actions (
    id              UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    report_id       UUID NOT NULL,
    report_community_id UUID NOT NULL,
    -- Client-generated idempotency key (signed in NIP-98 request body).
    request_id      UUID NOT NULL,
    -- Principal who claimed the report.
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    actor_role      TEXT NOT NULL CHECK (actor_role IN ('operator', 'moderator')),
    -- The enforcement action requested.
    action          TEXT NOT NULL,
    reason          TEXT,
    -- Timeout expiration for timeout actions; NULL otherwise.
    timeout_until   TIMESTAMPTZ,
    -- Durable state machine: pending → enforcing → succeeded|failed|cancelled.
    state           TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'enforcing', 'succeeded', 'failed', 'cancelled')),
    -- Step marker: the last durably committed mutation step (NULL = none yet).
    -- Values: 'mutation_committed' (core DB mutation done), 'artifacts_done' (tombstone/notice done).
    step_marker     TEXT CHECK (step_marker IN ('mutation_committed', 'artifacts_done')),
    -- Principal who cancelled a pre-mutation failed action; NULL until cancelled.
    -- Attributes the cancel transition on the action row itself, mirroring
    -- moderation_reports.resolved_by for report resolution.
    cancelled_by    BYTEA CHECK (cancelled_by IS NULL OR length(cancelled_by) = 32),
    -- Error from the last failure, if any.
    error_message   TEXT,
    -- Per-action exclusive lease (migration 0037): fences concurrent same-request
    -- retries and lets the recovery worker claim/re-drive stranded actions.
    action_lease_token      UUID,
    action_lease_expires_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Report-scoped idempotency: one action per (report, request_id).
    UNIQUE (report_community_id, report_id, request_id),
    FOREIGN KEY (report_community_id, report_id)
        REFERENCES moderation_reports (community_id, id)
);

CREATE INDEX idx_relay_admin_actions_report
    ON relay_admin_actions (report_community_id, report_id);
CREATE INDEX idx_relay_admin_actions_state
    ON relay_admin_actions (state)
    WHERE state IN ('pending', 'enforcing');
-- Recovery worker (migration 0037): find stranded actions by lease expiry.
CREATE INDEX idx_relay_admin_actions_lease
    ON relay_admin_actions (action_lease_expires_at)
    WHERE state IN ('pending', 'enforcing');

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('relay_admin_actions', 'deployment-global enforcement state machine; community_id is embedded in report FK');

-- ── Relay admin outbox (durable enforcement delivery) ────────────────────────
-- Transactional outbox for durable artifact/notice delivery.

CREATE TABLE relay_admin_outbox (
    id          UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    action_id   UUID NOT NULL REFERENCES relay_admin_actions(id),
    -- Delivery task type: 'tombstone' | 'system_message' | 'reporter_notice'.
    task_type   TEXT NOT NULL,
    -- Task payload (JSON).
    payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Lease-based delivery: held_by identifies the worker pod.
    held_by     TEXT,
    lease_expires_at TIMESTAMPTZ,
    -- Delivery state: pending → delivered | failed.
    state       TEXT NOT NULL DEFAULT 'pending'
                CHECK (state IN ('pending', 'delivered', 'failed')),
    -- Deduplication key: prevents re-creating an artifact after delivery.
    dedup_key   TEXT UNIQUE,
    error_message TEXT,
    -- Retryable delivery with backoff (migration 0037): failures reschedule via
    -- retry_after rather than terminating immediately.
    attempt_count INT NOT NULL DEFAULT 0,
    retry_after   TIMESTAMPTZ,
    -- Per-claim ownership fence (migration 0038): completion/failure updates
    -- require the token written at claim time, so a stale worker cannot overwrite
    -- a newer worker's terminal update.
    outbox_claim_token UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_relay_admin_outbox_action
    ON relay_admin_outbox (action_id);
CREATE INDEX idx_relay_admin_outbox_pending
    ON relay_admin_outbox (retry_after, created_at)
    WHERE state = 'pending';

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('relay_admin_outbox', 'deployment-global enforcement artifact delivery queue');

-- ── Relay operator audit (append-only roster mutation trail) ─────────────────
-- One row per PUT/DELETE /operators/{pubkey} mutation. The roster is the
-- deployment-wide root of trust and its mutations overwrite/remove in place;
-- this append-only trail records who granted, elevated, or revoked whom, and
-- when, so privilege changes are as auditable as the enforcement actions those
-- principals perform. Written only inside the upsert/delete transactions; no
-- UPDATE/DELETE path.

CREATE TABLE relay_operator_audit (
    id            UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_pubkey  BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    target_pubkey BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    op            TEXT NOT NULL CHECK (op IN ('grant', 'revoke')),
    prev_role     TEXT CHECK (prev_role IN ('operator', 'moderator')),
    new_role      TEXT CHECK (new_role IN ('operator', 'moderator')),
    -- created_at is wall-clock occurrence time (clock_timestamp()), informational
    -- only — not monotonic, so it never establishes order. `seq` is the sole
    -- chronology key: mutations write their audit row under the serializing lock,
    -- so identity order equals the true privilege chain. Reads use ORDER BY seq.
    created_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    seq           BIGINT GENERATED ALWAYS AS IDENTITY
);

CREATE INDEX idx_relay_operator_audit_target
    ON relay_operator_audit (target_pubkey, seq);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('relay_operator_audit', 'deployment-global append-only roster mutation audit trail; no community_id intentionally');


-- ============================================================================
-- NIP-FI core identity + base-lifecycle foundation (mirror of migration 0041).
-- The community_write_fence_excluded_table definition above already folds in
-- the NIP-FI ledger relations; the per-migration CREATE OR REPLACE bodies are
-- intentionally omitted here (desired state keeps one consolidated definition).
-- ============================================================================

-- The sole idempotency/result root shared by identity base lifecycle,
-- protected operations, and invalidation. Pre-authentication denials never
-- write this table. ExactReplay and IntentConflict are read-time observations,
-- not persisted outcomes.
CREATE TABLE authorization_operation_receipts (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    -- Core operation kinds: 1 enroll, 3 retire, 5 revoke, 6 rotate,
    -- 11 protected mutation, 12 invalidation. Extended lifecycle kinds
    -- (2 provision, 4 disable, 7 recover, 8 enable, 9 admission loss) and
    -- 10 operator are introduced by their owning later migrations.
    operation_kind SMALLINT NOT NULL CHECK (
        operation_kind IN (1, 3, 5, 6, 11, 12)
    ),
    actor_fingerprint BYTEA NOT NULL CHECK (octet_length(actor_fingerprint) = 32),
    -- 1 applied, 2 denied, 3 no-op.
    outcome_code SMALLINT NOT NULL CHECK (outcome_code IN (1, 2, 3)),
    result_digest BYTEA NOT NULL CHECK (octet_length(result_digest) = 32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    UNIQUE (community_id, operation_id, request_fingerprint),
    UNIQUE (
        community_id,
        operation_id,
        request_fingerprint,
        operation_kind,
        outcome_code
    ),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

-- Immutable monotonic local policy revisions. Enrollment modes are the closed
-- provider-free V1 set: 1 attested-key, 2 provisioned, 3 risk-labelled TOFU.
CREATE TABLE identity_enrollment_policies (
    community_id UUID NOT NULL REFERENCES communities(id),
    policy_revision BIGINT NOT NULL CHECK (policy_revision > 0),
    enrollment_mode SMALLINT NOT NULL CHECK (enrollment_mode IN (1, 2, 3)),
    policy_digest BYTEA NOT NULL CHECK (octet_length(policy_digest) = 32),
    effective_at TIMESTAMPTZ NOT NULL,
    -- Optional local binding-policy expiry. Federated token `exp` MUST NOT be
    -- copied here: token lifetime bounds an authorization lease, not this
    -- durable binding generation.
    expires_at TIMESTAMPTZ,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, policy_revision),
    CHECK (expires_at IS NULL OR effective_at < expires_at)
);

-- One row is one immutable binding generation. binding_version is allocated
-- from one non-cycling PostgreSQL identity sequence and is never changed or
-- reused. Explicit lifecycle may only retire the generation; X/Y denial
-- semantics live in immutable selector facts below, not alternate row states.
CREATE TABLE identity_bindings (
    community_id UUID NOT NULL REFERENCES communities(id),
    binding_id UUID NOT NULL,
    binding_version BIGINT GENERATED ALWAYS AS IDENTITY (
        START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1 NO CYCLE
    ),
    issuer TEXT COLLATE "C" NOT NULL CHECK (octet_length(issuer) BETWEEN 1 AND 2048),
    subject TEXT COLLATE "C" NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 2048),
    principal_fingerprint BYTEA NOT NULL CHECK (octet_length(principal_fingerprint) = 32),
    event_author_pubkey BYTEA NOT NULL CHECK (octet_length(event_author_pubkey) = 32),
    -- 1 active, 2 retired.
    binding_state SMALLINT NOT NULL CHECK (binding_state IN (1, 2)),
    lifecycle_revision BIGINT NOT NULL CHECK (lifecycle_revision IN (1, 2)),
    -- 1 attested-key, 2 provisioned, 3 risk-labelled TOFU.
    binding_provenance SMALLINT NOT NULL CHECK (binding_provenance IN (1, 2, 3)),
    policy_revision BIGINT NOT NULL CHECK (policy_revision > 0),
    -- Canonical evidence for the selected provenance. This is an assertion
    -- digest for attested/TOFU admission and a provisioning receipt digest for
    -- separately provisioned admission; it never stores credential bytes.
    enrollment_evidence_digest BYTEA NOT NULL CHECK (
        octet_length(enrollment_evidence_digest) = 32
    ),
    expires_at TIMESTAMPTZ,
    birth_history_id UUID NOT NULL,
    creation_operation_id UUID NOT NULL,
    creation_request_fingerprint BYTEA NOT NULL CHECK (
        octet_length(creation_request_fingerprint) = 32
    ),
    retirement_history_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, binding_id),
    UNIQUE (community_id, binding_version),
    UNIQUE (community_id, binding_id, binding_version),
    FOREIGN KEY (community_id, policy_revision)
        REFERENCES identity_enrollment_policies
            (community_id, policy_revision),
    CHECK (binding_version > 0),
    CHECK (binding_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (birth_history_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (creation_operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (expires_at IS NULL OR created_at < expires_at),
    CHECK (
        (binding_state = 1 AND lifecycle_revision = 1 AND retirement_history_id IS NULL)
        OR (binding_state = 2 AND lifecycle_revision = 2 AND retirement_history_id IS NOT NULL)
    )
);

-- State 1 is Active. Expiry is evaluated with authoritative PostgreSQL time
-- at read/finalization and is exclusive; it cannot appear in an index predicate.
CREATE UNIQUE INDEX identity_bindings_active_principal
    ON identity_bindings (community_id, issuer, subject)
    WHERE binding_state = 1;
CREATE INDEX identity_bindings_principal_fingerprint_lookup
    ON identity_bindings (community_id, principal_fingerprint)
    WHERE binding_state = 1;
CREATE UNIQUE INDEX identity_bindings_active_event_author
    ON identity_bindings (community_id, event_author_pubkey)
    WHERE binding_state = 1;
CREATE INDEX identity_bindings_current_lookup
    ON identity_bindings (community_id, event_author_pubkey, binding_state, expires_at);

-- The one canonical immutable lifecycle transition row for a successful or
-- no-op lifecycle operation. A transition can name an old generation, a new
-- successor generation, both (Rotate), or neither (a semantic no-op). It is not
-- a second result/effect engine: the shared receipt remains the sole persisted
-- operation outcome. Core transition kinds only: 1 enroll, 3 retire, 5 revoke,
-- 6 rotate.
CREATE TABLE identity_lifecycle_history (
    community_id UUID NOT NULL REFERENCES communities(id),
    history_id UUID NOT NULL,
    transition_kind SMALLINT NOT NULL CHECK (
        transition_kind IN (1, 3, 5, 6)
    ),
    -- Matches the shared receipt: 1 applied, 3 no-op.
    outcome_code SMALLINT NOT NULL CHECK (outcome_code IN (1, 3)),
    old_binding_id UUID,
    old_binding_version BIGINT CHECK (old_binding_version IS NULL OR old_binding_version > 0),
    old_prior_lifecycle_revision BIGINT CHECK (
        old_prior_lifecycle_revision IS NULL OR old_prior_lifecycle_revision IN (1, 2)
    ),
    old_prior_state SMALLINT CHECK (old_prior_state IS NULL OR old_prior_state IN (1, 2)),
    old_resulting_lifecycle_revision BIGINT CHECK (
        old_resulting_lifecycle_revision IS NULL OR old_resulting_lifecycle_revision IN (1, 2)
    ),
    old_resulting_state SMALLINT CHECK (
        old_resulting_state IS NULL OR old_resulting_state IN (1, 2)
    ),
    successor_binding_id UUID,
    successor_binding_version BIGINT CHECK (
        successor_binding_version IS NULL OR successor_binding_version > 0
    ),
    successor_lifecycle_revision BIGINT CHECK (
        successor_lifecycle_revision IS NULL OR successor_lifecycle_revision = 1
    ),
    successor_state SMALLINT CHECK (successor_state IS NULL OR successor_state = 1),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    transition_digest BYTEA NOT NULL CHECK (octet_length(transition_digest) = 32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, history_id),
    UNIQUE (community_id, operation_id),
    UNIQUE (community_id, history_id, operation_id, request_fingerprint),
    UNIQUE (
        community_id,
        history_id,
        successor_binding_id,
        successor_binding_version,
        operation_id,
        request_fingerprint
    ),
    UNIQUE (
        community_id,
        history_id,
        old_binding_id,
        old_binding_version,
        old_resulting_lifecycle_revision,
        old_resulting_state
    ),
    FOREIGN KEY (
        community_id,
        operation_id,
        request_fingerprint,
        transition_kind,
        outcome_code
    ) REFERENCES authorization_operation_receipts (
        community_id,
        operation_id,
        request_fingerprint,
        operation_kind,
        outcome_code
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, old_binding_id, old_binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, successor_binding_id, successor_binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (history_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (
        (old_binding_id IS NULL
            AND old_binding_version IS NULL
            AND old_prior_lifecycle_revision IS NULL
            AND old_prior_state IS NULL
            AND old_resulting_lifecycle_revision IS NULL
            AND old_resulting_state IS NULL)
        OR (old_binding_id IS NOT NULL
            AND old_binding_version IS NOT NULL
            AND old_prior_lifecycle_revision IS NOT NULL
            AND old_prior_state IS NOT NULL
            AND old_resulting_lifecycle_revision IS NOT NULL
            AND old_resulting_state IS NOT NULL)
    ),
    CHECK (
        (successor_binding_id IS NULL
            AND successor_binding_version IS NULL
            AND successor_lifecycle_revision IS NULL
            AND successor_state IS NULL)
        OR (successor_binding_id IS NOT NULL
            AND successor_binding_version IS NOT NULL
            AND successor_lifecycle_revision = 1
            AND successor_state = 1)
    ),
    CHECK (
        old_binding_id IS NULL
        OR successor_binding_id IS NULL
        OR old_binding_id <> successor_binding_id
    ),
    CHECK (
        old_binding_version IS NULL
        OR successor_binding_version IS NULL
        OR old_binding_version <> successor_binding_version
    ),
    -- Core lifecycle only ever moves Active/r1 to Retired/r2 for a named old
    -- generation. Extended re-enablement (recover/enable from Retired/r2) is a
    -- later migration's concern.
    CHECK (
        old_binding_id IS NULL
        OR (old_prior_lifecycle_revision = 1
            AND old_prior_state = 1
            AND old_resulting_lifecycle_revision = 2
            AND old_resulting_state = 2)
    ),
    CHECK (
        (outcome_code = 3
            AND old_binding_id IS NULL
            AND successor_binding_id IS NULL)
        OR (outcome_code = 1 AND (
            (transition_kind = 1
                AND old_binding_id IS NULL
                AND successor_binding_id IS NOT NULL)
            OR (transition_kind = 3
                AND old_binding_id IS NOT NULL
                AND successor_binding_id IS NULL)
            OR (transition_kind = 5
                AND successor_binding_id IS NULL)
            OR (transition_kind = 6
                AND old_binding_id IS NOT NULL
                AND successor_binding_id IS NOT NULL)
        ))
    )
);

CREATE INDEX identity_lifecycle_history_old_binding
    ON identity_lifecycle_history (community_id, old_binding_id, old_binding_version, recorded_at);
CREATE INDEX identity_lifecycle_history_successor_binding
    ON identity_lifecycle_history (
        community_id,
        successor_binding_id,
        successor_binding_version,
        recorded_at
    );

-- Circular birth/transition ordering is deliberate and fully deferred. Every
-- generation must commit with its exact birth transition, and a retired row
-- must commit with the exact transition that changed Active/r1 to Retired/r2.
ALTER TABLE identity_bindings
    ADD CONSTRAINT identity_bindings_exact_birth_history_fk
    FOREIGN KEY (
        community_id,
        birth_history_id,
        binding_id,
        binding_version,
        creation_operation_id,
        creation_request_fingerprint
    ) REFERENCES identity_lifecycle_history (
        community_id,
        history_id,
        successor_binding_id,
        successor_binding_version,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE identity_bindings
    ADD CONSTRAINT identity_bindings_exact_retirement_history_fk
    FOREIGN KEY (
        community_id,
        retirement_history_id,
        binding_id,
        binding_version,
        lifecycle_revision,
        binding_state
    ) REFERENCES identity_lifecycle_history (
        community_id,
        history_id,
        old_binding_id,
        old_binding_version,
        old_resulting_lifecycle_revision,
        old_resulting_state
    ) DEFERRABLE INITIALLY DEFERRED;

-- One immutable closed-scope fact table. Core selector kinds only:
-- 1 retired pair (P), 3 revoked key (Y). Both are permanent. The extended
-- disabled-identity (X) and pending-replacement (Q) selectors, and their
-- one-shot consumption, are introduced by the FI-LIFECYCLE migration.
CREATE TABLE identity_lifecycle_selectors (
    community_id UUID NOT NULL REFERENCES communities(id),
    selector_id UUID NOT NULL,
    selector_kind SMALLINT NOT NULL CHECK (selector_kind IN (1, 3)),
    selector_fingerprint BYTEA NOT NULL CHECK (octet_length(selector_fingerprint) = 32),
    fact_generation BIGINT NOT NULL CHECK (fact_generation > 0),
    principal_fingerprint BYTEA CHECK (
        principal_fingerprint IS NULL OR octet_length(principal_fingerprint) = 32
    ),
    event_author_pubkey BYTEA CHECK (
        event_author_pubkey IS NULL OR octet_length(event_author_pubkey) = 32
    ),
    binding_id UUID,
    binding_version BIGINT CHECK (binding_version IS NULL OR binding_version > 0),
    asserted_history_id UUID NOT NULL,
    selected_by_operation_id UUID NOT NULL,
    selected_by_request_fingerprint BYTEA NOT NULL CHECK (
        octet_length(selected_by_request_fingerprint) = 32
    ),
    selected_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, selector_id),
    UNIQUE (community_id, selector_id, selector_kind),
    UNIQUE (community_id, selector_kind, selector_fingerprint, fact_generation),
    FOREIGN KEY (
        community_id,
        asserted_history_id,
        selected_by_operation_id,
        selected_by_request_fingerprint
    ) REFERENCES identity_lifecycle_history (
        community_id,
        history_id,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        community_id,
        selected_by_operation_id,
        selected_by_request_fingerprint
    ) REFERENCES authorization_operation_receipts (
        community_id,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, binding_id, binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (selector_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (
        (selector_kind = 1
            AND fact_generation = 1
            AND principal_fingerprint IS NOT NULL
            AND event_author_pubkey IS NOT NULL
            AND binding_id IS NOT NULL
            AND binding_version IS NOT NULL)
        OR (selector_kind = 3
            AND fact_generation = 1
            AND principal_fingerprint IS NULL
            AND event_author_pubkey IS NOT NULL
            AND binding_id IS NULL
            AND binding_version IS NULL)
    )
);

CREATE UNIQUE INDEX identity_lifecycle_selectors_permanent_pair
    ON identity_lifecycle_selectors (community_id, binding_id, binding_version)
    WHERE selector_kind = 1;
CREATE UNIQUE INDEX identity_lifecycle_selectors_permanent_principal_key
    ON identity_lifecycle_selectors (
        community_id,
        principal_fingerprint,
        event_author_pubkey
    ) WHERE selector_kind = 1;
CREATE UNIQUE INDEX identity_lifecycle_selectors_permanent_key
    ON identity_lifecycle_selectors (community_id, event_author_pubkey)
    WHERE selector_kind = 3;
CREATE INDEX identity_lifecycle_selectors_principal_lookup
    ON identity_lifecycle_selectors
        (community_id, selector_kind, principal_fingerprint, fact_generation);
CREATE INDEX identity_lifecycle_selectors_key_lookup
    ON identity_lifecycle_selectors
        (community_id, selector_kind, event_author_pubkey, fact_generation);
CREATE INDEX identity_lifecycle_selectors_binding_lookup
    ON identity_lifecycle_selectors
        (community_id, selector_kind, binding_id, binding_version, fact_generation);
CREATE INDEX identity_lifecycle_selectors_asserted_history
    ON identity_lifecycle_selectors
        (community_id, asserted_history_id, selector_kind);

-- Serializes policy-revision inserts per community: each new revision must
-- strictly exceed the current maximum (FI-INV-06 — stable assertion policy
-- anchor; a backfilled or replayed revision is incoherent). The per-community
-- advisory lock prevents two concurrent writers from both passing a plain
-- SELECT MAX() check and committing conflicting revisions.
CREATE FUNCTION identity_enrollment_policy_revision_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    lock_key BIGINT;
    max_revision BIGINT;
BEGIN
    -- Acquire a per-community exclusive transaction-scoped advisory lock so
    -- that concurrent insertions serialize here. The key is a stable hash of
    -- the namespace string and the community_id bytes.
    lock_key := hashtextextended(
        'buzz:enrollment-policy-revision:v1:' || NEW.community_id::text,
        0
    );
    PERFORM pg_advisory_xact_lock(lock_key);

    SELECT MAX(policy_revision)
    INTO max_revision
    FROM identity_enrollment_policies
    WHERE community_id = NEW.community_id;

    IF max_revision IS NOT NULL
        AND NEW.policy_revision <= max_revision
    THEN
        RAISE EXCEPTION
            'policy_revision % does not strictly exceed current maximum % for community %',
            NEW.policy_revision, max_revision, NEW.community_id
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_enrollment_policy_revision_monotonic';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION nip_fi_reject_row_mutation_v1() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION '% is immutable', TG_TABLE_NAME
        USING ERRCODE = 'check_violation';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION nip_fi_reject_truncate_v1() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION '% cannot be truncated', TG_TABLE_NAME
        USING ERRCODE = 'check_violation';
END;
$$ LANGUAGE plpgsql;

-- Every binding/selector path derives the same domain-scoped coordinates and
-- takes their signed BIGINT advisory keys in numeric order. Typed transaction
-- APIs take these locks before row mutation; the triggers are the fail-closed
-- backstop for direct SQL.
CREATE FUNCTION identity_lifecycle_lock_coordinates_v1(
    locked_community_id UUID,
    locked_principal_fingerprint BYTEA,
    locked_event_author_pubkey BYTEA
) RETURNS VOID AS $$
DECLARE
    principal_lock_key BIGINT;
    event_author_lock_key BIGINT;
BEGIN
    IF locked_principal_fingerprint IS NOT NULL THEN
        principal_lock_key := hashtextextended(
            'buzz:identity-lifecycle-coordinate:v1:principal:'
                || locked_community_id::text || ':'
                || encode(locked_principal_fingerprint, 'hex'),
            0
        );
    END IF;
    IF locked_event_author_pubkey IS NOT NULL THEN
        event_author_lock_key := hashtextextended(
            'buzz:identity-lifecycle-coordinate:v1:key:'
                || locked_community_id::text || ':'
                || encode(locked_event_author_pubkey, 'hex'),
            0
        );
    END IF;

    IF principal_lock_key IS NOT NULL AND event_author_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(LEAST(principal_lock_key, event_author_lock_key));
        IF principal_lock_key <> event_author_lock_key THEN
            PERFORM pg_advisory_xact_lock(GREATEST(principal_lock_key, event_author_lock_key));
        END IF;
    ELSIF principal_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(principal_lock_key);
    ELSIF event_author_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(event_author_lock_key);
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_bindings_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    PERFORM identity_lifecycle_lock_coordinates_v1(
        NEW.community_id,
        NEW.principal_fingerprint,
        NEW.event_author_pubkey
    );
    IF NEW.binding_state <> 1
        OR NEW.lifecycle_revision <> 1
        OR NEW.retirement_history_id IS NOT NULL
    THEN
        RAISE EXCEPTION 'identity binding birth must be Active at lifecycle revision 1'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_birth_state';
    END IF;
    NEW.created_at := transaction_timestamp();
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_bindings_transition_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    PERFORM identity_lifecycle_lock_coordinates_v1(
        OLD.community_id,
        OLD.principal_fingerprint,
        OLD.event_author_pubkey
    );
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.binding_id IS DISTINCT FROM OLD.binding_id
        OR NEW.binding_version IS DISTINCT FROM OLD.binding_version
        OR NEW.issuer IS DISTINCT FROM OLD.issuer
        OR NEW.subject IS DISTINCT FROM OLD.subject
        OR NEW.principal_fingerprint IS DISTINCT FROM OLD.principal_fingerprint
        OR NEW.event_author_pubkey IS DISTINCT FROM OLD.event_author_pubkey
        OR NEW.binding_provenance IS DISTINCT FROM OLD.binding_provenance
        OR NEW.policy_revision IS DISTINCT FROM OLD.policy_revision
        OR NEW.enrollment_evidence_digest IS DISTINCT FROM OLD.enrollment_evidence_digest
        OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
        OR NEW.birth_history_id IS DISTINCT FROM OLD.birth_history_id
        OR NEW.creation_operation_id IS DISTINCT FROM OLD.creation_operation_id
        OR NEW.creation_request_fingerprint IS DISTINCT FROM OLD.creation_request_fingerprint
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'identity binding generation coordinates are immutable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_immutable_generation';
    END IF;
    IF OLD.binding_state <> 1
        OR OLD.lifecycle_revision <> 1
        OR OLD.retirement_history_id IS NOT NULL
        OR NEW.binding_state <> 2
        OR NEW.lifecycle_revision <> 2
        OR NEW.retirement_history_id IS NULL
        OR NEW.retirement_history_id = OLD.birth_history_id
    THEN
        RAISE EXCEPTION 'identity binding permits only Active/r1 to Retired/r2'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_active_to_retired';
    END IF;
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_history_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    NEW.recorded_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_binding_history_semantics_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    retirement identity_lifecycle_history%ROWTYPE;
BEGIN
    IF NEW.binding_state = 2 THEN
        SELECT * INTO STRICT retirement
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.retirement_history_id
          AND old_binding_id = NEW.binding_id
          AND old_binding_version = NEW.binding_version;
        IF retirement.outcome_code <> 1
            OR retirement.old_prior_lifecycle_revision <> 1
            OR retirement.old_prior_state <> 1
            OR retirement.old_resulting_lifecycle_revision <> 2
            OR retirement.old_resulting_state <> 2
        THEN
            RAISE EXCEPTION 'retired binding must reference its exact Active-to-Retired transition'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_bindings_retirement_history_semantics';
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_binding_birth_eligibility_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        WHERE selector.community_id = NEW.community_id
          AND (
            (selector.selector_kind = 1
                AND selector.principal_fingerprint = NEW.principal_fingerprint
                AND selector.event_author_pubkey = NEW.event_author_pubkey)
            OR (selector.selector_kind = 3
                AND selector.event_author_pubkey = NEW.event_author_pubkey)
          )
    ) THEN
        RAISE EXCEPTION 'binding birth conflicts with an effective lifecycle selector'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_birth_eligibility';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION authorization_operation_receipt_history_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    history_count BIGINT;
    expected_count BIGINT;
BEGIN
    SELECT count(*) INTO history_count
    FROM identity_lifecycle_history history
    WHERE history.community_id = NEW.community_id
      AND history.operation_id = NEW.operation_id;

    -- Core lifecycle receipts (enroll, retire, revoke, rotate) each require
    -- exactly one lifecycle-history row. Non-lifecycle receipts (protected
    -- mutation, invalidation) require none.
    expected_count := CASE
        WHEN NEW.operation_kind IN (1, 3, 5, 6) AND NEW.outcome_code IN (1, 3) THEN 1
        ELSE 0
    END;
    IF history_count <> expected_count THEN
        RAISE EXCEPTION 'operation receipt requires % lifecycle history row, found %',
            expected_count, history_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_receipt_history_cardinality';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_selector_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    NEW.selected_at := transaction_timestamp();
    PERFORM identity_lifecycle_lock_coordinates_v1(
        NEW.community_id,
        CASE WHEN NEW.selector_kind = 1 THEN NEW.principal_fingerprint END,
        NEW.event_author_pubkey
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_selector_history_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    history identity_lifecycle_history%ROWTYPE;
    old_binding identity_bindings%ROWTYPE;
BEGIN
    SELECT * INTO STRICT history
    FROM identity_lifecycle_history
    WHERE community_id = NEW.community_id
      AND history_id = NEW.asserted_history_id
      AND operation_id = NEW.selected_by_operation_id
      AND request_fingerprint = NEW.selected_by_request_fingerprint;

    IF history.old_binding_id IS NOT NULL THEN
        SELECT * INTO STRICT old_binding
        FROM identity_bindings
        WHERE community_id = history.community_id
          AND binding_id = history.old_binding_id
          AND binding_version = history.old_binding_version;
    END IF;

    -- A retired-pair (P) selector is asserted by retire, revoke, or rotate of a
    -- named old generation; a revoked-key (Y) selector by revoke.
    IF history.outcome_code <> 1
        OR (NEW.selector_kind = 1 AND (
            history.transition_kind NOT IN (3, 5, 6)
            OR history.old_binding_id IS DISTINCT FROM NEW.binding_id
            OR history.old_binding_version IS DISTINCT FROM NEW.binding_version
            OR old_binding.principal_fingerprint IS DISTINCT FROM NEW.principal_fingerprint
            OR old_binding.event_author_pubkey IS DISTINCT FROM NEW.event_author_pubkey
        ))
        OR (NEW.selector_kind = 3 AND (
            history.transition_kind <> 5
            OR (history.old_binding_id IS NOT NULL
                AND old_binding.event_author_pubkey
                    IS DISTINCT FROM NEW.event_author_pubkey)
        ))
    THEN
        RAISE EXCEPTION 'selector does not match its lifecycle transition'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_selector_history_semantics';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_transition_integrity_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    transition identity_lifecycle_history%ROWTYPE;
    old_binding_state SMALLINT;
    asserted_p BIGINT;
    asserted_y BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'identity_lifecycle_history' THEN
        transition := NEW;
    ELSIF TG_TABLE_NAME = 'identity_lifecycle_selectors' THEN
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.asserted_history_id;
    ELSE
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = CASE
              WHEN NEW.binding_state = 2 THEN NEW.retirement_history_id
              ELSE NEW.birth_history_id
          END;
    END IF;

    SELECT
        count(*) FILTER (WHERE selector_kind = 1),
        count(*) FILTER (WHERE selector_kind = 3)
    INTO asserted_p, asserted_y
    FROM identity_lifecycle_selectors
    WHERE community_id = transition.community_id
      AND asserted_history_id = transition.history_id;

    IF transition.old_binding_id IS NOT NULL THEN
        SELECT binding_state INTO STRICT old_binding_state
        FROM identity_bindings
        WHERE community_id = transition.community_id
          AND binding_id = transition.old_binding_id
          AND binding_version = transition.old_binding_version;
        IF old_binding_state <> 2 THEN
            RAISE EXCEPTION 'lifecycle transition old binding must be retired at commit'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_lifecycle_transition_integrity';
        END IF;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        JOIN identity_bindings active
          ON active.community_id = selector.community_id
         AND active.binding_state = 1
         AND (
            (selector.selector_kind = 1
                AND active.principal_fingerprint = selector.principal_fingerprint
                AND active.event_author_pubkey = selector.event_author_pubkey)
            OR (selector.selector_kind = 3
                AND active.event_author_pubkey = selector.event_author_pubkey)
         )
        WHERE selector.community_id = transition.community_id
    ) THEN
        RAISE EXCEPTION 'effective lifecycle selector conflicts with an active binding'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_transition_integrity';
    END IF;

    IF transition.outcome_code = 3 THEN
        IF asserted_p + asserted_y <> 0 THEN
            RAISE EXCEPTION 'no-op lifecycle transition cannot create selector facts'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_lifecycle_transition_integrity';
        END IF;
        RETURN NULL;
    END IF;

    -- Core selector companions per transition:
    --   enroll  (1): none
    --   retire  (3): exactly one P
    --   revoke  (5): one Y always; one P when a named old generation is removed
    --   rotate  (6): exactly one P (old generation retired)
    IF (transition.transition_kind = 1
            AND (asserted_p, asserted_y) <> (0, 0))
        OR (transition.transition_kind = 3
            AND (asserted_p, asserted_y) <> (1, 0))
        OR (transition.transition_kind = 5 AND (
            (transition.old_binding_id IS NOT NULL
                AND (asserted_p, asserted_y) <> (1, 1))
            OR (transition.old_binding_id IS NULL
                AND (asserted_p, asserted_y) <> (0, 1))
        ))
        OR (transition.transition_kind = 6
            AND (asserted_p, asserted_y) <> (1, 0))
    THEN
        RAISE EXCEPTION 'lifecycle transition has incomplete or forbidden selector companions'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_transition_integrity';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER identity_bindings_insert_guard
    BEFORE INSERT ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION identity_bindings_insert_guard_v1();
CREATE TRIGGER identity_bindings_transition_guard
    BEFORE UPDATE ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION identity_bindings_transition_guard_v1();
CREATE CONSTRAINT TRIGGER identity_bindings_history_semantics
    AFTER INSERT OR UPDATE ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_binding_history_semantics_guard_v1();
CREATE CONSTRAINT TRIGGER identity_bindings_birth_eligibility
    AFTER INSERT ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_binding_birth_eligibility_guard_v1();
CREATE CONSTRAINT TRIGGER identity_bindings_transition_integrity
    AFTER INSERT OR UPDATE ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();
CREATE TRIGGER identity_bindings_no_delete
    BEFORE DELETE ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_bindings_no_truncate
    BEFORE TRUNCATE ON identity_bindings
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_lifecycle_history_insert_guard
    BEFORE INSERT ON identity_lifecycle_history
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_history_insert_guard_v1();
CREATE CONSTRAINT TRIGGER authorization_operation_receipt_history_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_receipt_history_guard_v1();
CREATE CONSTRAINT TRIGGER identity_lifecycle_transition_integrity
    AFTER INSERT ON identity_lifecycle_history
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

CREATE TRIGGER identity_lifecycle_selector_insert_guard
    BEFORE INSERT ON identity_lifecycle_selectors
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_selector_insert_guard_v1();
CREATE CONSTRAINT TRIGGER identity_lifecycle_selector_history_semantics
    AFTER INSERT ON identity_lifecycle_selectors
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_selector_history_guard_v1();
CREATE CONSTRAINT TRIGGER identity_lifecycle_selector_transition_integrity
    AFTER INSERT ON identity_lifecycle_selectors
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

CREATE TRIGGER authorization_operation_receipts_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_receipts
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operation_receipts_no_truncate
    BEFORE TRUNCATE ON authorization_operation_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_enrollment_policies_revision_guard
    BEFORE INSERT ON identity_enrollment_policies
    FOR EACH ROW EXECUTE FUNCTION identity_enrollment_policy_revision_guard_v1();
CREATE TRIGGER identity_enrollment_policies_immutable
    BEFORE UPDATE OR DELETE ON identity_enrollment_policies
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_enrollment_policies_no_truncate
    BEFORE TRUNCATE ON identity_enrollment_policies
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_lifecycle_history_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_history
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_lifecycle_history_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_history
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_lifecycle_selectors_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_selectors
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_lifecycle_selectors_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_selectors
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();


-- ============================================================================
-- NIP-FI final-admission foundation (mirror of migration 0042).
-- ============================================================================

CREATE TABLE authorization_invalidation_domains (
    community_id UUID NOT NULL PRIMARY KEY REFERENCES communities(id),
    current_generation BIGINT NOT NULL CHECK (current_generation >= 0),
    activated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp()
);

-- Closed selectors: 1 principal, 2 Nostr key, 3 binding, 4 session, 5 domain,
-- 6 configuration revision. Selector 7 (delegated relationship) and its
-- relationship-revision floor are deferred to the FI-DELEG migration.
CREATE TABLE authorization_invalidation_floors (
    community_id UUID NOT NULL REFERENCES communities(id),
    selector_kind SMALLINT NOT NULL CHECK (selector_kind IN (1, 2, 3, 4, 5, 6)),
    selector_fingerprint BYTEA NOT NULL CHECK (octet_length(selector_fingerprint) = 32),
    floor_generation BIGINT NOT NULL CHECK (floor_generation > 0),
    binding_version_floor BIGINT CHECK (binding_version_floor IS NULL OR binding_version_floor > 0),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, selector_kind, selector_fingerprint),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (selector_kind = 3 AND binding_version_floor IS NOT NULL)
        OR (selector_kind <> 3 AND binding_version_floor IS NULL)
    )
);

-- Protected-object kinds: 1 domain, 2 channel, 3 repository, 4 media,
-- 5 moderation target, 6 audio session. Kind 7 is retired: current binding
-- status is connection-local evidence and never a durable protected object.
CREATE TABLE authorization_authority_epochs (
    community_id UUID NOT NULL REFERENCES communities(id),
    object_kind SMALLINT NOT NULL CHECK (object_kind IN (1, 2, 3, 4, 5, 6)),
    object_key BYTEA NOT NULL CHECK (octet_length(object_key) = 32),
    authority_epoch BIGINT NOT NULL CHECK (authority_epoch > 0),
    fence BYTEA NOT NULL CHECK (
        octet_length(fence) = 32 AND fence <> decode(repeat('00', 32), 'hex')
    ),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, object_kind, object_key),
    UNIQUE (
        community_id,
        object_kind,
        object_key,
        authority_epoch,
        fence,
        operation_id,
        request_fingerprint
    ),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED
);

-- Direct-final current authority for a protected object. The authorization
-- lease itself is sealed in memory and dies on restart; this durable row is the
-- exact source re-fenced immediately before a protected mutation or emission.
CREATE TABLE protected_object_authority (
    community_id UUID NOT NULL REFERENCES communities(id),
    object_kind SMALLINT NOT NULL CHECK (object_kind IN (1, 2, 3, 4, 5, 6)),
    object_key BYTEA NOT NULL CHECK (octet_length(object_key) = 32),
    capability SMALLINT NOT NULL CHECK (
        capability IN (
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
            15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
            28, 29
        )
    ),
    actor_pubkey BYTEA NOT NULL CHECK (octet_length(actor_pubkey) = 32),
    binding_id UUID NOT NULL,
    binding_version BIGINT NOT NULL CHECK (binding_version > 0),
    policy_revision BIGINT NOT NULL CHECK (policy_revision > 0),
    invalidation_generation BIGINT NOT NULL CHECK (invalidation_generation >= 0),
    authority_epoch BIGINT NOT NULL CHECK (authority_epoch > 0),
    fence BYTEA NOT NULL CHECK (
        octet_length(fence) = 32 AND fence <> decode(repeat('00', 32), 'hex')
    ),
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    PRIMARY KEY (community_id, object_kind, object_key),
    FOREIGN KEY (community_id, binding_id, binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        community_id,
        object_kind,
        object_key,
        authority_epoch,
        fence,
        operation_id,
        request_fingerprint
    ) REFERENCES authorization_authority_epochs (
        community_id,
        object_kind,
        object_key,
        authority_epoch,
        fence,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED,
    CHECK (issued_at < expires_at)
);

-- Explicit immutable-capacity policy required by Enforce mode. Hard ceilings
-- match buzz-auth; installation limits must be sized explicitly below them.
-- V1 has no online pruning/export/reset workflow.
CREATE TABLE authorization_event_capacity (
    community_id UUID NOT NULL PRIMARY KEY REFERENCES communities(id),
    max_events_per_domain BIGINT NOT NULL CONSTRAINT authorization_event_capacity_max_events CHECK (
        max_events_per_domain BETWEEN 1 AND 10000
    ),
    max_bytes_per_domain BIGINT NOT NULL CONSTRAINT authorization_event_capacity_max_bytes CHECK (
        max_bytes_per_domain BETWEEN 1 AND 16777216
    ),
    max_envelope_bytes INTEGER NOT NULL CONSTRAINT authorization_event_capacity_max_envelope CHECK (
        max_envelope_bytes BETWEEN 1 AND 16384
    ),
    retained_event_count BIGINT NOT NULL DEFAULT 0 CHECK (retained_event_count >= 0),
    retained_envelope_bytes BIGINT NOT NULL DEFAULT 0 CHECK (retained_envelope_bytes >= 0),
    -- 1 healthy, 2 audit unavailable/exhausted. Recovery/reset is not a V1
    -- online workflow; enabled runtime latches failure when insertion aborts.
    health_state SMALLINT NOT NULL DEFAULT 1 CHECK (health_state IN (1, 2)),
    failure_code SMALLINT CHECK (failure_code IS NULL OR failure_code IN (1, 2, 3)),
    failure_observed_at TIMESTAMPTZ,
    configured_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    CHECK (max_envelope_bytes <= max_bytes_per_domain),
    CHECK (retained_event_count <= max_events_per_domain),
    CHECK (retained_envelope_bytes <= max_bytes_per_domain),
    CHECK (
        (health_state = 1 AND failure_code IS NULL AND failure_observed_at IS NULL)
        OR (health_state = 2 AND failure_code IS NOT NULL AND failure_observed_at IS NOT NULL)
    )
);

-- Durable versioned pseudonymous authorization envelope. event_kind:
-- 1 enrolled, 2 revoked, 3 rotated, 6 retired, 9 operator denied,
-- 10 protected allowed, 11 protected denied, 14 invalidation advanced.
-- The extended-lifecycle audit kinds (4 recovered, 5 principal enabled,
-- 7 principal disabled, 8 admission lost) are deferred to the FI-LIFECYCLE
-- migration, matching 0041's core lifecycle carve. Kinds 12 and 13 are
-- retired: kind 24244 publication/withdrawal is ephemeral connection state and
-- never a durable authorization event.
CREATE TABLE authorization_events (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id UUID NOT NULL,
    schema_version SMALLINT NOT NULL DEFAULT 1 CHECK (schema_version = 1),
    event_kind SMALLINT NOT NULL CHECK (
        event_kind IN (1, 2, 3, 6, 9, 10, 11, 14)
    ),
    outcome_code SMALLINT NOT NULL CHECK (outcome_code IN (1, 2, 3, 4, 5)),
    reason_code SMALLINT NOT NULL CHECK (
        reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    ),
    actor_kind SMALLINT NOT NULL CHECK (actor_kind IN (1, 2, 3, 4)),
    actor_fingerprint BYTEA CHECK (
        actor_fingerprint IS NULL OR octet_length(actor_fingerprint) = 32
    ),
    subject_fingerprint BYTEA CHECK (
        subject_fingerprint IS NULL OR octet_length(subject_fingerprint) = 32
    ),
    -- Always retains attempted operation identity. Only unresolved pre-auth
    -- event kind 9 omits the canonical receipt fingerprint; authenticated
    -- OperatorDenied events remain linked to their exact canonical receipt.
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA CHECK (
        request_fingerprint IS NULL OR octet_length(request_fingerprint) = 32
    ),
    correlation_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    -- Redaction-safe pre-authentication denial identity. Present and non-zero
    -- for unresolved pre-auth kind-9 events (actor_kind = 4); NULL for
    -- authenticated kind-9 events (actor_kind 1-3) and all other event kinds.
    -- Binds the event to the exact denial attempt's semantic_fingerprint
    -- (intent_digest) for exact replay.
    semantic_fingerprint BYTEA CHECK (
        semantic_fingerprint IS NULL OR octet_length(semantic_fingerprint) = 32
    ),
    occurred_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    canonical_envelope BYTEA NOT NULL CONSTRAINT authorization_events_envelope_size CHECK (
        octet_length(canonical_envelope) BETWEEN 1 AND 16384
    ),
    envelope_digest BYTEA NOT NULL CHECK (octet_length(envelope_digest) = 32),
    PRIMARY KEY (community_id, event_id),
    UNIQUE (community_id, event_id, operation_id),
    UNIQUE (community_id, event_id, event_kind, operation_id),
    UNIQUE (community_id, operation_id, event_kind, attempt_id),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (event_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (correlation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (attempt_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (
        (actor_kind = 4 AND event_kind = 9 AND request_fingerprint IS NULL)
        OR (actor_kind IN (1, 2, 3) AND request_fingerprint IS NOT NULL)
    ),
    CHECK (
        (actor_kind = 4 AND actor_fingerprint IS NULL AND subject_fingerprint IS NULL)
        OR (actor_kind IN (1, 2, 3) AND actor_fingerprint IS NOT NULL)
    ),
    -- Unresolved pre-auth kind-9 events (actor_kind = 4) carry a non-zero
    -- semantic_fingerprint; authenticated kind-9 events (actor_kind 1-3) and
    -- all other event kinds must not.
    CHECK (
        (event_kind = 9 AND actor_kind = 4 AND semantic_fingerprint IS NOT NULL
            AND semantic_fingerprint <> decode(repeat('00', 32), 'hex'))
        OR (event_kind = 9 AND actor_kind IN (1, 2, 3) AND semantic_fingerprint IS NULL)
        OR (event_kind <> 9 AND semantic_fingerprint IS NULL)
    )
);

-- Credential-free pre-authentication denial attempts. The five-column key is
-- exact replay identity; no row or FK occupies canonical operation/result,
-- effect, authority, approval, or consumption state.
CREATE TABLE authorization_authentication_denial_attempts (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    correlation_id UUID NOT NULL,
    semantic_fingerprint BYTEA NOT NULL CHECK (octet_length(semantic_fingerprint) = 32),
    denial_reason SMALLINT NOT NULL CHECK (denial_reason IN (1, 2, 3)),
    expected_revision BIGINT NOT NULL CHECK (expected_revision > 0),
    action SMALLINT NOT NULL CHECK (action IN (1, 2, 3, 4, 5, 6, 7, 8)),
    reason_code SMALLINT NOT NULL CHECK (
        reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    ),
    attempt_id UUID NOT NULL,
    audit_event_id UUID NOT NULL,
    audit_event_kind SMALLINT NOT NULL DEFAULT 9 CHECK (audit_event_kind = 9),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (
        community_id,
        operation_id,
        correlation_id,
        semantic_fingerprint,
        denial_reason
    ),
    UNIQUE (community_id, audit_event_id),
    FOREIGN KEY (community_id, audit_event_id, audit_event_kind, operation_id)
        REFERENCES authorization_events (community_id, event_id, event_kind, operation_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, operation_id, audit_event_kind, attempt_id)
        REFERENCES authorization_events (community_id, operation_id, event_kind, attempt_id)
        DEFERRABLE INITIALLY DEFERRED,
    -- Canonical denial_reason ↔ reason_code binding: MissingCredential(1)↔Missing(2),
    -- InvalidCredential(2)↔Invalid(3), Unauthenticated(3)↔Unauthenticated(4).
    CONSTRAINT authorization_denial_reason_reason_code_binding CHECK (
        (denial_reason = 1 AND reason_code = 2)
        OR (denial_reason = 2 AND reason_code = 3)
        OR (denial_reason = 3 AND reason_code = 4)
    ),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (correlation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (attempt_id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

-- Exact per-operation authority-version attribution for restore. Empty
-- manifests are valid; every stored component must advance strictly.
CREATE TABLE authorization_operation_version_delta_manifests (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    component_count INTEGER NOT NULL CHECK (component_count BETWEEN 0 AND 1024),
    before_digest BYTEA NOT NULL CHECK (octet_length(before_digest) = 32),
    after_digest BYTEA NOT NULL CHECK (octet_length(after_digest) = 32),
    manifest_digest BYTEA NOT NULL CHECK (octet_length(manifest_digest) = 32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    UNIQUE (community_id, operation_id, request_fingerprint),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED
);

-- component_kind: 1 binding version, 2 policy revision,
-- 3 invalidation generation, 4 authority epoch. Kind 6 (delegated-relationship
-- revision) is deferred to the FI-DELEG migration and kind 7 (lifecycle-selector
-- generation) to the FI-LIFECYCLE migration. Kind 5 is retired with durable
-- client-status revisions; retained kinds keep their original identities.
CREATE TABLE authorization_operation_version_deltas (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    component_kind SMALLINT NOT NULL CHECK (component_kind IN (1, 2, 3, 4)),
    component_key BYTEA NOT NULL CHECK (octet_length(component_key) = 32),
    before_version BIGINT NOT NULL CHECK (before_version >= 0),
    after_version BIGINT NOT NULL,
    component_digest BYTEA NOT NULL CHECK (octet_length(component_digest) = 32),
    PRIMARY KEY (community_id, operation_id, component_kind, component_key),
    FOREIGN KEY (community_id, operation_id)
        REFERENCES authorization_operation_version_delta_manifests
            (community_id, operation_id),
    CHECK (after_version > before_version)
);

CREATE FUNCTION authorization_event_capacity_before_insert_v1() RETURNS TRIGGER AS $$
DECLARE
    policy authorization_event_capacity%ROWTYPE;
    envelope_bytes BIGINT;
BEGIN
    SELECT * INTO policy
    FROM authorization_event_capacity
    WHERE community_id = NEW.community_id
    FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'authorization event capacity policy missing'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_policy_required';
    END IF;
    IF policy.health_state <> 1 THEN
        RAISE EXCEPTION 'authorization audit is unavailable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_health';
    END IF;

    envelope_bytes := octet_length(NEW.canonical_envelope);
    IF envelope_bytes > policy.max_envelope_bytes
        OR policy.retained_event_count + 1 > policy.max_events_per_domain
        OR policy.retained_envelope_bytes + envelope_bytes > policy.max_bytes_per_domain
    THEN
        -- The INSERT and protected mutation abort together. The runtime maps
        -- this stable constraint to typed CapacityExhausted and latches audit
        -- health outside the rolled-back transaction.
        RAISE EXCEPTION 'authorization event capacity exhausted'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_exhausted';
    END IF;

    UPDATE authorization_event_capacity
    SET retained_event_count = retained_event_count + 1,
        retained_envelope_bytes = retained_envelope_bytes + envelope_bytes,
        updated_at = transaction_timestamp()
    WHERE community_id = NEW.community_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_events_capacity
    BEFORE INSERT ON authorization_events
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_before_insert_v1();

CREATE FUNCTION authorization_invalidation_domain_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.activated_at IS DISTINCT FROM OLD.activated_at
        OR NEW.current_generation <= OLD.current_generation
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization invalidation activation/generation cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_invalidation_domains_monotonic
    BEFORE UPDATE ON authorization_invalidation_domains
    FOR EACH ROW EXECUTE FUNCTION authorization_invalidation_domain_guard_v1();
CREATE TRIGGER authorization_invalidation_domains_no_delete
    BEFORE DELETE ON authorization_invalidation_domains
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_invalidation_domains_no_truncate
    BEFORE TRUNCATE ON authorization_invalidation_domains
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_invalidation_floor_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.selector_kind IS DISTINCT FROM OLD.selector_kind
        OR NEW.selector_fingerprint IS DISTINCT FROM OLD.selector_fingerprint
        OR NEW.floor_generation < OLD.floor_generation
        OR COALESCE(NEW.binding_version_floor, 0) < COALESCE(OLD.binding_version_floor, 0)
        OR (
            NEW.floor_generation = OLD.floor_generation
            AND COALESCE(NEW.binding_version_floor, 0)
                = COALESCE(OLD.binding_version_floor, 0)
        )
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization invalidation floor cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_invalidation_floors_monotonic
    BEFORE UPDATE ON authorization_invalidation_floors
    FOR EACH ROW EXECUTE FUNCTION authorization_invalidation_floor_guard_v1();
CREATE TRIGGER authorization_invalidation_floors_no_delete
    BEFORE DELETE ON authorization_invalidation_floors
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_invalidation_floors_no_truncate
    BEFORE TRUNCATE ON authorization_invalidation_floors
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_authority_epoch_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.authority_epoch <= OLD.authority_epoch
        OR NEW.fence IS NOT DISTINCT FROM OLD.fence
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization authority epoch cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_authority_epochs_monotonic
    BEFORE UPDATE ON authorization_authority_epochs
    FOR EACH ROW EXECUTE FUNCTION authorization_authority_epoch_guard_v1();
CREATE TRIGGER authorization_authority_epochs_no_delete
    BEFORE DELETE ON authorization_authority_epochs
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_authority_epochs_no_truncate
    BEFORE TRUNCATE ON authorization_authority_epochs
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_event_capacity_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.max_events_per_domain IS DISTINCT FROM OLD.max_events_per_domain
        OR NEW.max_bytes_per_domain IS DISTINCT FROM OLD.max_bytes_per_domain
        OR NEW.max_envelope_bytes IS DISTINCT FROM OLD.max_envelope_bytes
        OR NEW.configured_at IS DISTINCT FROM OLD.configured_at
        OR NEW.retained_event_count < OLD.retained_event_count
        OR NEW.retained_envelope_bytes < OLD.retained_envelope_bytes
        OR NEW.updated_at < OLD.updated_at
        OR (OLD.health_state = 2 AND (
            NEW.health_state <> 2
            OR NEW.failure_code IS DISTINCT FROM OLD.failure_code
            OR NEW.failure_observed_at IS DISTINCT FROM OLD.failure_observed_at
        ))
        OR (OLD.health_state = 1 AND NEW.health_state = 1 AND (
            NEW.failure_code IS NOT NULL OR NEW.failure_observed_at IS NOT NULL
        ))
    THEN
        RAISE EXCEPTION 'authorization event capacity cannot be reset online'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION protected_object_authority_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.authority_epoch <= OLD.authority_epoch
        OR NEW.fence IS NOT DISTINCT FROM OLD.fence
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.issued_at <= OLD.issued_at
    THEN
        RAISE EXCEPTION 'protected authority replacement requires a new operation and epoch'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_event_capacity_monotonic
    BEFORE UPDATE ON authorization_event_capacity
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_guard_v1();
CREATE TRIGGER authorization_event_capacity_no_delete
    BEFORE DELETE ON authorization_event_capacity
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_event_capacity_no_truncate
    BEFORE TRUNCATE ON authorization_event_capacity
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER authorization_events_immutable
    BEFORE UPDATE OR DELETE ON authorization_events
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_events_no_truncate
    BEFORE TRUNCATE ON authorization_events
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER authorization_authentication_denial_attempts_immutable
    BEFORE UPDATE OR DELETE ON authorization_authentication_denial_attempts
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_authentication_denial_attempts_no_truncate
    BEFORE TRUNCATE ON authorization_authentication_denial_attempts
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

-- Bidirectional deferred guard: a kind-9 (pre-authentication denial) audit
-- event must commit with exactly one denial attempt; a denial attempt must
-- commit with its audit event present, kind-9, and matching semantic
-- coordinates (correlation_id, reason_code, and semantic_fingerprint). Both
-- directions deferred so event and attempt may be inserted in any order inside
-- one transaction. The static denial_reason↔reason_code mapping is enforced
-- by an immediate CHECK on the denial attempt table; the guard enforces the
-- matching semantic coordinates between event and attempt.
CREATE FUNCTION authorization_denial_attempt_guard_v1()
RETURNS TRIGGER AS $$
DECLARE
    found_event_kind SMALLINT;
    found_actor_kind SMALLINT;
    found_request_fingerprint BYTEA;
    found_correlation_id UUID;
    found_reason_code SMALLINT;
    found_semantic_fingerprint BYTEA;
    attempt_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_events' THEN
        -- Firing from the event side: only unresolved pre-auth kind-9 events
        -- (actor_kind = 4) require a denial attempt row. Authenticated
        -- OperatorDenied events (actor_kind 1-3) have a canonical receipt and
        -- no denial attempt.
        IF NEW.event_kind <> 9 OR NEW.actor_kind <> 4 THEN
            RETURN NULL;
        END IF;

        SELECT count(*) INTO attempt_count
        FROM authorization_authentication_denial_attempts
        WHERE community_id = NEW.community_id
          AND audit_event_id = NEW.event_id;

        IF attempt_count <> 1 THEN
            RAISE EXCEPTION
                'kind-9 audit event requires exactly one denial attempt, found %',
                attempt_count
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_event_cardinality';
        END IF;

        -- Verify semantic coordinates match between event and denial attempt.
        SELECT correlation_id, reason_code, semantic_fingerprint
        INTO found_correlation_id, found_reason_code, found_semantic_fingerprint
        FROM authorization_authentication_denial_attempts
        WHERE community_id = NEW.community_id
          AND audit_event_id = NEW.event_id;

        IF found_correlation_id IS DISTINCT FROM NEW.correlation_id THEN
            RAISE EXCEPTION
                'denial attempt correlation_id % does not match event correlation_id % for event %',
                found_correlation_id, NEW.correlation_id, NEW.event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_semantic_binding';
        END IF;

        IF found_reason_code IS DISTINCT FROM NEW.reason_code THEN
            RAISE EXCEPTION
                'denial attempt reason_code % does not match event reason_code % for event %',
                found_reason_code, NEW.reason_code, NEW.event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_semantic_binding';
        END IF;

        IF found_semantic_fingerprint IS DISTINCT FROM NEW.semantic_fingerprint THEN
            RAISE EXCEPTION
                'denial attempt semantic_fingerprint does not match event semantic_fingerprint for event %',
                NEW.event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_semantic_binding';
        END IF;
    ELSE
        -- Firing from the denial-attempt side: verify the audit event is the
        -- unresolved pre-auth kind-9 shape (actor_kind = 4, null receipt
        -- fingerprint) and that exactly one denial attempt references it.
        SELECT event_kind, actor_kind, request_fingerprint,
               correlation_id, reason_code, semantic_fingerprint
        INTO found_event_kind, found_actor_kind, found_request_fingerprint,
             found_correlation_id, found_reason_code,
             found_semantic_fingerprint
        FROM authorization_events
        WHERE community_id = NEW.community_id
          AND event_id = NEW.audit_event_id;

        IF NOT FOUND THEN
            RAISE EXCEPTION
                'denial attempt references non-existent audit event %',
                NEW.audit_event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_event_kind';
        END IF;

        IF found_event_kind <> 9 THEN
            RAISE EXCEPTION
                'denial attempt audit event must be kind 9, got %',
                found_event_kind
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_event_kind';
        END IF;

        -- The referenced event must be the unresolved pre-auth shape: actor_kind
        -- 4 with a null receipt fingerprint. Attaching a denial attempt to an
        -- authenticated OperatorDenied (actor_kind 1-3) would violate the
        -- credential-free pre-authentication contract.
        IF found_actor_kind <> 4 OR found_request_fingerprint IS NOT NULL THEN
            RAISE EXCEPTION
                'denial attempt must reference an unresolved pre-auth kind-9 event '
                '(actor_kind 4, null request_fingerprint); got actor_kind % '
                'and request_fingerprint % for event %',
                found_actor_kind,
                CASE WHEN found_request_fingerprint IS NULL THEN 'null' ELSE 'non-null' END,
                NEW.audit_event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_event_kind';
        END IF;

        -- Verify semantic coordinates match.
        IF found_correlation_id IS DISTINCT FROM NEW.correlation_id THEN
            RAISE EXCEPTION
                'denial attempt correlation_id % does not match event correlation_id % for event %',
                NEW.correlation_id, found_correlation_id, NEW.audit_event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_semantic_binding';
        END IF;

        IF found_reason_code IS DISTINCT FROM NEW.reason_code THEN
            RAISE EXCEPTION
                'denial attempt reason_code % does not match event reason_code % for event %',
                NEW.reason_code, found_reason_code, NEW.audit_event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_semantic_binding';
        END IF;

        IF found_semantic_fingerprint IS DISTINCT FROM NEW.semantic_fingerprint THEN
            RAISE EXCEPTION
                'denial attempt semantic_fingerprint does not match event semantic_fingerprint for event %',
                NEW.audit_event_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_semantic_binding';
        END IF;

        SELECT count(*) INTO attempt_count
        FROM authorization_authentication_denial_attempts
        WHERE community_id = NEW.community_id
          AND audit_event_id = NEW.audit_event_id;

        IF attempt_count <> 1 THEN
            RAISE EXCEPTION
                'exactly one denial attempt must reference audit event %, found %',
                NEW.audit_event_id, attempt_count
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denial_attempt_event_cardinality';
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER authorization_denial_attempt_event_cardinality
    AFTER INSERT ON authorization_events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_denial_attempt_guard_v1();

CREATE CONSTRAINT TRIGGER authorization_denial_event_attempt_cardinality
    AFTER INSERT ON authorization_authentication_denial_attempts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_denial_attempt_guard_v1();

CREATE FUNCTION authorization_operation_version_delta_cardinality_guard_v1()
RETURNS TRIGGER AS $$
DECLARE
    manifest authorization_operation_version_delta_manifests%ROWTYPE;
    actual_component_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_version_delta_manifests' THEN
        manifest := NEW;
    ELSE
        SELECT * INTO STRICT manifest
        FROM authorization_operation_version_delta_manifests
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id
        FOR NO KEY UPDATE;
    END IF;

    SELECT count(*) INTO actual_component_count
    FROM authorization_operation_version_deltas
    WHERE community_id = manifest.community_id
      AND operation_id = manifest.operation_id;

    IF actual_component_count <> manifest.component_count THEN
        RAISE EXCEPTION 'operation version manifest declares % components, found %',
            manifest.component_count, actual_component_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_version_delta_cardinality';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER authorization_operation_version_delta_manifest_cardinality
    AFTER INSERT ON authorization_operation_version_delta_manifests
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_version_delta_cardinality_guard_v1();
CREATE CONSTRAINT TRIGGER authorization_operation_version_delta_component_cardinality
    AFTER INSERT ON authorization_operation_version_deltas
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_version_delta_cardinality_guard_v1();

CREATE TRIGGER authorization_operation_version_delta_manifests_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_version_delta_manifests
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operation_version_delta_manifests_no_truncate
    BEFORE TRUNCATE ON authorization_operation_version_delta_manifests
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER authorization_operation_version_deltas_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_version_deltas
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operation_version_deltas_no_truncate
    BEFORE TRUNCATE ON authorization_operation_version_deltas
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER protected_object_authority_no_delete
    BEFORE DELETE ON protected_object_authority
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER protected_object_authority_no_truncate
    BEFORE TRUNCATE ON protected_object_authority
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();
CREATE TRIGGER protected_object_authority_strict_replacement
    BEFORE UPDATE ON protected_object_authority
    FOR EACH ROW EXECUTE FUNCTION protected_object_authority_guard_v1();

-- Canonical admission keeps its complete logical intent and the closed,
-- credential-free application result beside the immutable receipt. This is
-- what lets an identical request replay reconstruct the same typed result
-- without repeating membership or other application DML. Object kinds match
-- protected_object_authority: 1 domain, 2 channel, 3 repository, 4 media,
-- 5 moderation target, 6 audio session.
CREATE TABLE authorization_admission_results (
    community_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    semantic_fingerprint BYTEA NOT NULL CHECK (
        octet_length(semantic_fingerprint) = 32
        AND semantic_fingerprint <> decode(repeat('00', 32), 'hex')
    ),
    object_kind SMALLINT NOT NULL CHECK (object_kind BETWEEN 1 AND 6),
    object_key BYTEA NOT NULL CHECK (
        octet_length(object_key) = 32
        AND object_key <> decode(repeat('00', 32), 'hex')
    ),
    application_type BYTEA CHECK (
        application_type IS NULL
        OR (octet_length(application_type) = 32
            AND application_type <> decode(repeat('00', 32), 'hex'))
    ),
    application_version SMALLINT CHECK (application_version > 0),
    application_code SMALLINT CHECK (application_code > 0),
    application_payload BYTEA CHECK (
        application_payload IS NULL OR octet_length(application_payload) <= 4096
    ),
    application_intent_digest BYTEA CHECK (
        application_intent_digest IS NULL
        OR (octet_length(application_intent_digest) = 32
            AND application_intent_digest <> decode(repeat('00', 32), 'hex'))
    ),
    application_effect_digest BYTEA CHECK (
        application_effect_digest IS NULL
        OR (octet_length(application_effect_digest) = 32
            AND application_effect_digest <> decode(repeat('00', 32), 'hex'))
    ),
    application_result_digest BYTEA CHECK (
        application_result_digest IS NULL
        OR (octet_length(application_result_digest) = 32
            AND application_result_digest <> decode(repeat('00', 32), 'hex'))
    ),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint),
    CHECK (
        (application_type IS NULL
            AND application_version IS NULL
            AND application_code IS NULL
            AND application_payload IS NULL
            AND application_intent_digest IS NULL
            AND application_effect_digest IS NULL
            AND application_result_digest IS NULL)
        OR (application_type IS NOT NULL
            AND application_version IS NOT NULL
            AND application_code IS NOT NULL
            AND application_payload IS NOT NULL
            AND application_intent_digest IS NOT NULL
            AND application_effect_digest IS NOT NULL
            AND application_result_digest IS NOT NULL)
    )
);

CREATE TRIGGER authorization_admission_results_no_update
    BEFORE UPDATE OR DELETE ON authorization_admission_results
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_admission_results_no_truncate
    BEFORE TRUNCATE ON authorization_admission_results
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

-- Bidirectional deferred cardinality guard: a kind-11 (protected-mutation)
-- receipt must commit with exactly one admission result; an admission result
-- must commit against a kind-11 receipt. Deferred so receipt and result may
-- be inserted in any order inside one transaction.
CREATE FUNCTION authorization_admission_result_guard_v1()
RETURNS TRIGGER AS $$
DECLARE
    receipt authorization_operation_receipts%ROWTYPE;
    result_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_receipts' THEN
        receipt := NEW;
    ELSE
        -- Firing from authorization_admission_results: look up the receipt.
        SELECT * INTO receipt
        FROM authorization_operation_receipts
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id;
        IF NOT FOUND THEN
            -- FK on the result table already guards the non-existent receipt
            -- case; this path should not occur in normal operation.
            RAISE EXCEPTION
                'admission result references non-existent receipt for operation %',
                NEW.operation_id
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_admission_result_receipt_kind';
        END IF;
    END IF;

    -- Non-kind-11 receipts require no admission result.
    IF receipt.operation_kind <> 11 THEN
        -- If this fired from the result side and the receipt is not kind 11,
        -- the result is attaching to the wrong receipt kind.
        IF TG_TABLE_NAME = 'authorization_admission_results' THEN
            RAISE EXCEPTION
                'admission result may only attach to a kind-11 (protected-mutation) receipt, got kind %',
                receipt.operation_kind
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_admission_result_receipt_kind';
        END IF;
        RETURN NULL;
    END IF;

    SELECT count(*) INTO result_count
    FROM authorization_admission_results
    WHERE community_id = receipt.community_id
      AND operation_id = receipt.operation_id;

    IF result_count <> 1 THEN
        RAISE EXCEPTION
            'kind-11 receipt requires exactly one admission result, found %',
            result_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_admission_result_cardinality';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER authorization_admission_result_receipt_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_admission_result_guard_v1();

CREATE CONSTRAINT TRIGGER authorization_admission_result_result_cardinality
    AFTER INSERT ON authorization_admission_results
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_admission_result_guard_v1();

-- Every successful/no-op core lifecycle receipt has exactly one privacy-safe
-- audit event with the closed transition-kind mapping. Both directions are
-- deferred so receipt, history, event, selectors, and binding may be inserted
-- in any order inside one transaction but can never commit partially. The
-- extended-lifecycle operation kinds (2 provision, 4 disable, 7 recover,
-- 8 enable, 9 admission loss) and their event kinds arrive with the
-- FI-LIFECYCLE migration; here the mapping covers only enroll/retire/revoke/
-- rotate. Non-lifecycle receipts (protected mutation, invalidation) carry no
-- audit-event cardinality requirement.
CREATE FUNCTION authorization_operation_receipt_event_guard_v1()
RETURNS TRIGGER AS $$
DECLARE
    receipt authorization_operation_receipts%ROWTYPE;
    expected_event_kind SMALLINT;
    matching_event_count BIGINT;
    expected_event_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_receipts' THEN
        receipt := NEW;
    ELSE
        SELECT * INTO receipt
        FROM authorization_operation_receipts
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id;
        IF NOT FOUND THEN
            -- Credential-free pre-authentication denials intentionally have no
            -- canonical receipt. Their separate FK/shape guards still run.
            RETURN NULL;
        END IF;
    END IF;

    expected_event_kind := CASE receipt.operation_kind
        WHEN 1 THEN 1  -- enroll
        WHEN 3 THEN 6  -- retire
        WHEN 5 THEN 2  -- revoke
        WHEN 6 THEN 3  -- rotate
        ELSE NULL
    END;
    IF expected_event_kind IS NULL THEN
        RETURN NULL;
    END IF;

    -- Only applied (outcome_code = 1) and no-op (outcome_code = 3) lifecycle
    -- receipts require exactly one paired success-transition event. A denied
    -- lifecycle receipt (outcome_code = 2) requires zero events from the
    -- complete core lifecycle success-transition class (kinds 1, 2, 3, 6:
    -- enrolled, revoked, rotated, retired). Forbidding only the mapped kind
    -- would allow a wrong-kind transition event to attach to the denied receipt,
    -- which is equally a contradictory durable fact. Legitimate audit/denial
    -- events of other kinds (e.g., authenticated kind 9) remain allowed.
    -- Other outcome codes (4, 5) are not core lifecycle outcomes; skip.
    IF receipt.outcome_code IN (1, 3) THEN
        SELECT
            count(*),
            count(*) FILTER (WHERE event_kind = expected_event_kind)
        INTO matching_event_count, expected_event_count
        FROM authorization_events
        WHERE community_id = receipt.community_id
          AND operation_id = receipt.operation_id
          AND request_fingerprint = receipt.request_fingerprint;

        IF matching_event_count <> 1 OR expected_event_count <> 1 THEN
            RAISE EXCEPTION
                'lifecycle receipt requires exactly one event kind %, found % total and % expected',
                expected_event_kind, matching_event_count, expected_event_count
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_operation_receipt_event_cardinality';
        END IF;
    ELSIF receipt.outcome_code = 2 THEN
        SELECT count(*) FILTER (WHERE event_kind IN (1, 2, 3, 6))
        INTO expected_event_count
        FROM authorization_events
        WHERE community_id = receipt.community_id
          AND operation_id = receipt.operation_id
          AND request_fingerprint = receipt.request_fingerprint;

        IF expected_event_count <> 0 THEN
            RAISE EXCEPTION
                'denied lifecycle receipt must not have any core success-transition event '
                '(kinds 1/2/3/6); found % — contradictory durable facts are not permitted',
                expected_event_count
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'authorization_denied_lifecycle_receipt_no_success_event';
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER authorization_operation_receipt_event_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_receipt_event_guard_v1();

CREATE CONSTRAINT TRIGGER authorization_event_receipt_cardinality
    AFTER INSERT ON authorization_events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_receipt_event_guard_v1();
