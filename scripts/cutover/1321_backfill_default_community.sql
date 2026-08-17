-- scripts/cutover/1321_backfill_default_community.sql
--
-- One-off cutover: single-community (pre-1321) Postgres -> multi-tenant (1321)
-- schema. Assigns EVERY existing row to one default community derived from the
-- deployment host.
--
-- ─────────────────────────────────────────────────────────────────────────────
-- STRATEGY: rename pre-1321 objects aside into a `legacy` schema, rebuild the
--           correct 1321 schema in `public` from 0001 VERBATIM, copy data
--           forward, drop `legacy`.
--
--   Why not an in-place ALTER ("just add community_id and re-key")? The 1321
-- schema differs from pre-1321 by far MORE than a prepended community_id, and
-- hand-transcribing the difference is how silent isolation defects get in (local
-- validation caught an in-place draft that left 5 tables single-tenant-keyed and
-- dropped ~13 FKs). The real delta:
--     * channels gains  ttl_seconds, ttl_deadline
--     * events   gains  not_before, delivered_at, search_tsv (GENERATED) + GIN
--     * EVERY scoped table is re-keyed to a community-leading PK/UNIQUE
--     * EVERY hot-path index is replaced with a community-leading one
--     * channels gains a community_id immutability trigger
--     * 6 brand-new tables (communities, scheduled_workflow_fires, relay_members,
--       archived_identities, audit_log, _operator_global_tables)
--
--   So we do NOT describe the target by hand. We let 1321's own
-- migrations/0001_initial_schema.sql define it, verbatim, into a clean `public`.
-- The result is structurally identical to a fresh 1321 DB BY CONSTRUCTION.
--
--   The one obstacle to running the from-scratch 0001 against an existing DB is
-- name collisions: 0001 issues bare `CREATE TYPE <enum>` (Postgres has no
-- CREATE TYPE IF NOT EXISTS) and `CREATE TABLE <name>`. We clear `public` of
-- those names first by renaming the pre-1321 tables AND their enum types into a
-- `legacy` schema. `CREATE EXTENSION IF NOT EXISTS pgcrypto` in 0001 is a no-op
-- (idempotent), and pgcrypto stays in public untouched.
--
--   Phases (all in ONE transaction; all-or-nothing):
--     A  CREATE SCHEMA legacy; move every pre-1321 table + enum type into it
--     B  \i the repo's 0001  -> full correct 1321 schema in clean public
--     C  create the one default community from :host
--     D  INSERT ... SELECT default_community, <explicit cols> from each legacy.*
--        (enum cols cast ::text::public.<enum>; search_tsv regenerates; new
--        columns default NULL)
--     E  DROP SCHEMA legacy CASCADE
-- ─────────────────────────────────────────────────────────────────────────────
--
-- PRECONDITIONS
--   * DB is the pre-1321 single-community schema (no community_id columns, no
--     `communities` table). Guard below refuses to run otherwise.
--   * Run psql with this script via -f (not piped on stdin): the Phase B
--     include below uses \ir, which resolves relative to THIS file, so the
--     repo's migrations/0001 is found regardless of the operator's cwd.
--   * TAKE A pg_dump / PVC SNAPSHOT FIRST. No down-path; the snapshot IS the
--     rollback.
--
-- USAGE  (run from anywhere; \ir below resolves the schema include relative to
--         this file, so cwd does not matter)
--   psql "$DATABASE_URL" \
--        -v host="'sprout-oss.stage.blox.sqprod.co'" \
--        -v ON_ERROR_STOP=1 \
--        -f scripts/cutover/1321_backfill_default_community.sql
--
-- After commit: boot the 1321 relay with BUZZ_AUTO_MIGRATE=false. The schema is
-- already correct; the relay's idempotent boot-time
-- ensure_configured_community / allowlist->relay_members backfill finds this
-- community and no-ops. (If you instead want the relay's sqlx migrator to
-- consider 0001 "applied", seed its _sqlx_migrations row — see the runbook;
-- not required for correct operation.)

\set ON_ERROR_STOP on

BEGIN;

-- ── Guard: refuse to run on anything but pristine pre-1321 ───────────────────
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables
               WHERE table_schema='public' AND table_name='communities') THEN
        RAISE EXCEPTION
          'communities table already exists -- DB is not pristine pre-1321 '
          '(already migrated). Refusing to run.';
    END IF;
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema='public' AND column_name='community_id') THEN
        RAISE EXCEPTION
          'a community_id column already exists -- DB is not pristine pre-1321. '
          'Refusing to run.';
    END IF;
    IF EXISTS (SELECT 1 FROM information_schema.schemata
               WHERE schema_name='legacy') THEN
        RAISE EXCEPTION
          'schema legacy already exists -- leftover from a failed run. '
          'Drop it (DROP SCHEMA legacy CASCADE) and retry.';
    END IF;
END
$guard$;

-- ── Phase A: move pre-1321 tables + enum types into `legacy` ─────────────────
CREATE SCHEMA legacy;

-- Tables (partitioned parents move with their partitions via SET SCHEMA on the
-- parent? -- no: SET SCHEMA does NOT move partition children. So we move the
-- parent AND each partition. Simpler and safe: move every base/partitioned/
-- partition relation owned in public that is one of our pre-1321 tables.)
DO $move_tables$
DECLARE r RECORD;
BEGIN
    FOR r IN
        SELECT c.relname
        FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public'
          AND c.relkind IN ('r','p')          -- ordinary + partitioned parents
        ORDER BY c.relkind DESC                -- move partition children before parents? not needed
    LOOP
        EXECUTE format('ALTER TABLE public.%I SET SCHEMA legacy', r.relname);
    END LOOP;
END
$move_tables$;

-- Enum types (must leave public so 0001 can CREATE TYPE with the same names).
DO $move_types$
DECLARE r RECORD;
BEGIN
    FOR r IN
        SELECT t.typname
        FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
        WHERE n.nspname = 'public' AND t.typtype = 'e'
    LOOP
        EXECUTE format('ALTER TYPE public.%I SET SCHEMA legacy', r.typname);
    END LOOP;
END
$move_types$;

-- ── Phase B: the authoritative 1321 schema, verbatim, into clean public ──────
-- \ir = include relative to THIS script's directory (scripts/cutover/), so the
-- repo's migrations/0001 resolves regardless of the operator's cwd.
\ir ../../migrations/0001_initial_schema.sql

-- ── Phase C: the one default community, normalized like the relay does ───────
INSERT INTO public.communities (host)
VALUES (rtrim(lower(:host), '.'));
SELECT id AS default_community
  FROM public.communities
 WHERE lower(host) = rtrim(lower(:host), '.') \gset

-- ── Phase D: copy data forward ───────────────────────────────────────────────
-- Source legacy.*, target public.*. Enum columns cross a type boundary
-- (legacy.<enum> -> public.<enum>); same labels, so ::text::public.<enum> is
-- lossless. search_tsv omitted (GENERATED); new-in-1321 columns omitted (NULL).

INSERT INTO public.channels
    (community_id, id, name, channel_type, visibility, description, canvas,
     created_by, created_at, updated_at, archived_at, deleted_at, nip29_group_id,
     topic_required, max_members, topic, topic_set_by, topic_set_at, purpose,
     purpose_set_by, purpose_set_at, participant_hash)
SELECT :'default_community', id, name,
     channel_type::text::public.channel_type,
     visibility::text::public.channel_visibility,
     description, canvas, created_by, created_at, updated_at, archived_at,
     deleted_at, nip29_group_id, topic_required, max_members, topic, topic_set_by,
     topic_set_at, purpose, purpose_set_by, purpose_set_at, participant_hash
FROM legacy.channels;

INSERT INTO public.channel_members
    (community_id, channel_id, pubkey, role, joined_at, invited_by, removed_at,
     removed_by, hidden_at)
SELECT :'default_community', channel_id, pubkey,
     role::text::public.member_role,
     joined_at, invited_by, removed_at, removed_by, hidden_at
FROM legacy.channel_members;

INSERT INTO public.users
    (community_id, pubkey, nip05_handle, display_name, avatar_url, about,
     agent_type, capabilities, okta_user_id, created_at, updated_at,
     deactivated_at, metadata_event_id, agent_owner_pubkey, channel_add_policy)
SELECT :'default_community', pubkey, nip05_handle, display_name, avatar_url, about,
     agent_type, capabilities, okta_user_id, created_at, updated_at,
     deactivated_at, metadata_event_id, agent_owner_pubkey,
     channel_add_policy::text::public.channel_add_policy
FROM legacy.users;

INSERT INTO public.events
    (community_id, id, pubkey, created_at, kind, tags, content, sig,
     received_at, channel_id, deleted_at, d_tag)
SELECT :'default_community', id, pubkey, created_at, kind, tags, content, sig,
     received_at, channel_id, deleted_at, d_tag
FROM legacy.events;

INSERT INTO public.event_mentions
    (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind)
SELECT :'default_community', pubkey_hex, event_id, event_created_at, channel_id, event_kind
FROM legacy.event_mentions;

INSERT INTO public.subscriptions
    (community_id, id, owner_pubkey, filter_kinds, filter_authors,
     filter_channel_ids, filter_since, filter_until, delivery_method,
     delivery_url, status, pause_reason, delivered_count, error_count,
     created_at, updated_at)
SELECT :'default_community', id, owner_pubkey, filter_kinds, filter_authors,
     filter_channel_ids, filter_since, filter_until,
     delivery_method::text::public.delivery_method, delivery_url,
     status::text::public.subscription_status,
     pause_reason::text::public.pause_reason,
     delivered_count, error_count, created_at, updated_at
FROM legacy.subscriptions;

-- delivery_log.id is GENERATED ALWAYS AS IDENTITY; preserve original ids.
INSERT INTO public.delivery_log
    (community_id, id, subscription_id, event_id, method, delivered_at, success,
     http_status, error_message, attempt_number)
OVERRIDING SYSTEM VALUE
SELECT :'default_community', id, subscription_id, event_id,
     method::text::public.delivery_method, delivered_at, success,
     http_status, error_message, attempt_number
FROM legacy.delivery_log;

INSERT INTO public.workflows
    (community_id, id, name, owner_pubkey, channel_id, definition,
     definition_hash, status, enabled, created_at, updated_at)
SELECT :'default_community', id, name, owner_pubkey, channel_id, definition,
     definition_hash, status::text::public.workflow_status, enabled,
     created_at, updated_at
FROM legacy.workflows;

INSERT INTO public.workflow_runs
    (community_id, id, workflow_id, status, trigger_event_id, current_step,
     execution_trace, trigger_context, started_at, completed_at, error_message,
     created_at)
SELECT :'default_community', id, workflow_id,
     status::text::public.run_status, trigger_event_id, current_step,
     execution_trace, trigger_context, started_at, completed_at, error_message,
     created_at
FROM legacy.workflow_runs;

INSERT INTO public.workflow_approvals
    (community_id, token, workflow_id, run_id, step_id, step_index, approver_spec,
     status, approver_pubkey, note, granted_at, denied_at, expires_at, created_at)
SELECT :'default_community', token, workflow_id, run_id, step_id, step_index,
     approver_spec, status::text::public.approval_status, approver_pubkey,
     note, granted_at, denied_at, expires_at, created_at
FROM legacy.workflow_approvals;

INSERT INTO public.api_tokens
    (community_id, id, token_hash, owner_pubkey, name, scopes, channel_ids,
     created_at, expires_at, last_used_at, revoked_at, revoked_by,
     created_by_self_mint)
SELECT :'default_community', id, token_hash, owner_pubkey, name, scopes, channel_ids,
     created_at, expires_at, last_used_at, revoked_at, revoked_by,
     created_by_self_mint
FROM legacy.api_tokens;

-- rate_limit_violations: OPERATOR-GLOBAL; community_id nullable attribution.
-- id is IDENTITY; preserve.
INSERT INTO public.rate_limit_violations
    (id, community_id, pubkey, violation_at, limit_type, limit_value,
     actual_value, action_taken)
OVERRIDING SYSTEM VALUE
SELECT id, :'default_community', pubkey, violation_at, limit_type, limit_value,
     actual_value, action_taken
FROM legacy.rate_limit_violations;

INSERT INTO public.thread_metadata
    (community_id, event_created_at, event_id, channel_id, parent_event_id,
     parent_event_created_at, root_event_id, root_event_created_at, depth,
     reply_count, descendant_count, last_reply_at, broadcast)
SELECT :'default_community', event_created_at, event_id, channel_id, parent_event_id,
     parent_event_created_at, root_event_id, root_event_created_at, depth,
     reply_count, descendant_count, last_reply_at, broadcast
FROM legacy.thread_metadata;

INSERT INTO public.reactions
    (community_id, event_created_at, event_id, pubkey, emoji, created_at,
     removed_at, reaction_event_id)
SELECT :'default_community', event_created_at, event_id, pubkey, emoji, created_at,
     removed_at, reaction_event_id
FROM legacy.reactions;

INSERT INTO public.pubkey_allowlist
    (community_id, pubkey, added_by, added_at, note)
SELECT :'default_community', pubkey, added_by, added_at, note
FROM legacy.pubkey_allowlist;

-- ── Phase E: retire the pre-1321 objects ─────────────────────────────────────
DROP SCHEMA legacy CASCADE;

COMMIT;

-- ── Post-commit sanity (run manually; not part of the txn) ───────────────────
-- SELECT count(*) FROM communities;                       -- expect 1
-- SELECT count(*) FROM events WHERE community_id IS NULL;  -- expect 0
-- SELECT to_regnamespace('legacy');                        -- expect NULL (gone)
