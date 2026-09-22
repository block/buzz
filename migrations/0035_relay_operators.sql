-- Deployment-global relay operator roster (Phase 2 admin auth: role-based).
--
-- This table tracks deployment-level principals (operators and moderators)
-- staffed via the admin API. Config-backed operators (RELAY_OPERATOR_PUBKEYS,
-- RELAY_OWNER_PUBKEY owner-fallback) are never seeded here — they are
-- authoritative in config and outrank any DB row.
--
-- Global (no community_id): operators span all tenants, so tenant-scoping
-- would be a misnomer. Registered in _operator_global_tables and in the
-- hardcoded parser list in crates/buzz-db/src/migration.rs.

CREATE TABLE relay_operators (
    pubkey      BYTEA NOT NULL PRIMARY KEY CHECK (length(pubkey) = 32),
    role        TEXT NOT NULL CHECK (role IN ('operator', 'moderator')),
    added_by    BYTEA NOT NULL CHECK (length(added_by) = 32),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('relay_operators',
     'deployment-global operator/moderator roster; no community_id intentionally');

-- ── Schema amendments to existing tables ──────────────────────────────────────

-- moderation_actions: add actor_authority column to record whether the acting
-- principal was a community member or a deployment operator/moderator.
-- Default 'community' for all existing rows.
ALTER TABLE moderation_actions
    ADD COLUMN actor_authority TEXT
        NOT NULL DEFAULT 'community'
        CHECK (actor_authority IN ('community', 'relay_operator', 'relay_moderator'));

-- moderation_reports: add processing status for HTTP enforcement claim.
-- The CAS on status='open' AND active_action_id IS NULL ensures only one
-- HTTP enforcement can claim a report at a time; legacy 9044 resolvers CAS
-- on status='open' which fails safely against a processing report.
ALTER TABLE moderation_reports
    DROP CONSTRAINT moderation_reports_status_check,
    ADD CONSTRAINT moderation_reports_status_check
        CHECK (status IN ('open', 'processing', 'resolved', 'dismissed', 'escalated'));

-- active_action_id is non-null when the report is in 'processing' state.
-- Only relay_admin_actions rows (Phase 2) may set this field; the column
-- is the lease claim for exactly-once enforcement.
ALTER TABLE moderation_reports
    ADD COLUMN active_action_id UUID;

-- product_feedback: add operator-managed status column.
ALTER TABLE product_feedback
    ADD COLUMN status TEXT NOT NULL DEFAULT 'new'
        CHECK (status IN ('new', 'reviewed', 'archived'));
