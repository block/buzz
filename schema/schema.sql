-- Desired-state database manifest.
--
-- `tables/public/<table>.sql` declares one table per file: its CREATE TABLE
-- and the CREATE INDEX statements on it, the one-table-per-file shape SchemaBot
-- reads. `tables/` is the SchemaBot schema directory and `public` is its
-- namespace, so nothing else may live under it.
--
-- Everything SchemaBot cannot apply lives in this directory's other
-- subdirectories: `types/`, `functions/`, `partitions/`, `triggers/` (including
-- write-fence attachment), `seeds/`, and the `community_write_fence.sql`
-- bootstrap. This file composes all of them into one fresh database, in load
-- order: types → functions → tables → partitions → triggers → seeds → fence.
--
-- `./bin/pgschema apply --file schema/schema.sql` resolves the `\i` includes
-- relative to this file (it refuses `..`). `pgschema` skips INSERTs and some
-- storage parameters, so every apply caller must then run
-- `scripts/reconcile-schema-after-pgschema.sql`. Raw `psql` resolves `\i`
-- against the working directory instead, so run it from here:
-- `cd schema && psql -f schema.sql`.
--
-- Adding a table: add `tables/public/<table>.sql` and include it below after
-- the tables it references; put its partitions, triggers, and seed rows in the
-- matching subdirectories and include those too. This file holds only `\i`
-- lines and `CREATE EXTENSION`. Outside `tables/public/`, no file may create,
-- alter or drop a table or index, except `CREATE TABLE ... PARTITION OF` in
-- `partitions/`. The layout test enforces both.

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
--   3. `channels.community_id` is immutable (triggers/channels.sql; no UPDATE path).
--   4. Operator-global tables are named in the explicit allowlist, not implied.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- types
\i types/channel_type.sql
\i types/channel_visibility.sql
\i types/member_role.sql
\i types/workflow_status.sql
\i types/run_status.sql
\i types/approval_status.sql
\i types/delivery_method.sql
\i types/subscription_status.sql
\i types/pause_reason.sql
\i types/channel_add_policy.sql

-- functions
\i functions/channels_community_id_immutable.sql
\i functions/enqueue_push_match_job.sql
\i functions/refresh_channel_ttl_after_event_insert.sql
\i functions/guard_channel_roster_snapshot.sql
\i functions/events_created_at_floor_guard.sql
\i functions/prevent_community_deletion_request_retargeting.sql
\i functions/prevent_community_deletion_approval_removal.sql
\i functions/protect_community_deletion_manifest_keys.sql
\i functions/community_deletion_lock_key.sql
\i functions/community_write_fence_excluded_table.sql
\i functions/community_write_allowed.sql
\i functions/assert_community_write_allowed.sql
\i functions/enforce_community_write_fence.sql
\i functions/enforce_community_tombstone.sql
\i functions/attach_community_write_fence.sql

-- tables
\i tables/public/communities.sql
\i tables/public/channels.sql
\i tables/public/channel_members.sql
\i tables/public/users.sql
\i tables/public/personal_read_accounts.sql
\i tables/public/personal_read_frontiers.sql
\i tables/public/events.sql
\i tables/public/event_mentions.sql
\i tables/public/subscriptions.sql
\i tables/public/delivery_log.sql
\i tables/public/workflows.sql
\i tables/public/workflow_runs.sql
\i tables/public/workflow_approvals.sql
\i tables/public/scheduled_workflow_fires.sql
\i tables/public/api_tokens.sql
\i tables/public/rate_limit_violations.sql
\i tables/public/thread_metadata.sql
\i tables/public/reactions.sql
\i tables/public/pubkey_allowlist.sql
\i tables/public/relay_members.sql
\i tables/public/join_policy_acceptances.sql
\i tables/public/relay_invites.sql
\i tables/public/archived_identities.sql
\i tables/public/audit_log.sql
\i tables/public/moderation_actions.sql
\i tables/public/moderation_reports.sql
\i tables/public/community_bans.sql
\i tables/public/_operator_global_tables.sql
\i tables/public/git_repo_names.sql
\i tables/public/parameterized_event_watermarks.sql
\i tables/public/product_feedback.sql
\i tables/public/push_leases.sql
\i tables/public/push_wake_outbox.sql
\i tables/public/push_match_queue.sql
\i tables/public/push_gateway_challenges.sql
\i tables/public/push_gateway_installations.sql
\i tables/public/push_gateway_delegations.sql
\i tables/public/push_gateway_endpoint_quotas.sql
\i tables/public/push_gateway_delivery_auth_replays.sql
\i tables/public/push_gateway_delivery_request_replays.sql
\i tables/public/replica_heartbeat.sql
\i tables/public/community_deletion_requests.sql
\i tables/public/community_deletion_approvals.sql
\i tables/public/community_deletion_checkpoints.sql
\i tables/public/community_deletion_manifest_keys.sql
\i tables/public/storage_taxonomy_sweeps.sql
\i tables/public/community_serving_write_leases.sql
\i tables/public/community_deletion_executor_heartbeats.sql
\i tables/public/relay_operators.sql
\i tables/public/relay_admin_actions.sql
\i tables/public/relay_admin_outbox.sql
\i tables/public/operator_listener_pubkeys.sql
\i tables/public/operator_listener_outbox.sql
\i tables/public/relay_operator_audit.sql
\i tables/public/storage_accounting_snapshots.sql
\i tables/public/artifact_heads.sql
\i tables/public/artifact_revisions.sql

-- partitions
\i partitions/events.sql
\i partitions/delivery_log.sql

-- triggers
\i triggers/channels.sql
\i triggers/events.sql
\i triggers/community_deletion_requests.sql
\i triggers/community_deletion_approvals.sql
\i triggers/community_deletion_manifest_keys.sql
\i triggers/communities.sql
\i triggers/api_tokens.sql
\i triggers/archived_identities.sql
\i triggers/audit_log.sql
\i triggers/channel_members.sql
\i triggers/community_bans.sql
\i triggers/delivery_log.sql
\i triggers/event_mentions.sql
\i triggers/git_repo_names.sql
\i triggers/join_policy_acceptances.sql
\i triggers/moderation_actions.sql
\i triggers/moderation_reports.sql
\i triggers/parameterized_event_watermarks.sql
\i triggers/personal_read_accounts.sql
\i triggers/personal_read_frontiers.sql
\i triggers/pubkey_allowlist.sql
\i triggers/push_leases.sql
\i triggers/push_match_queue.sql
\i triggers/push_wake_outbox.sql
\i triggers/reactions.sql
\i triggers/relay_invites.sql
\i triggers/relay_members.sql
\i triggers/scheduled_workflow_fires.sql
\i triggers/subscriptions.sql
\i triggers/thread_metadata.sql
\i triggers/users.sql
\i triggers/workflow_approvals.sql
\i triggers/workflow_runs.sql
\i triggers/workflows.sql
\i triggers/artifact_heads.sql
\i triggers/artifact_revisions.sql

-- seeds
\i seeds/communities.sql
\i seeds/rate_limit_violations.sql
\i seeds/_operator_global_tables.sql
\i seeds/product_feedback.sql
\i seeds/push_gateway_challenges.sql
\i seeds/push_gateway_installations.sql
\i seeds/push_gateway_delegations.sql
\i seeds/push_gateway_endpoint_quotas.sql
\i seeds/push_gateway_delivery_auth_replays.sql
\i seeds/push_gateway_delivery_request_replays.sql
\i seeds/replica_heartbeat.sql
\i seeds/community_deletion_requests.sql
\i seeds/community_deletion_approvals.sql
\i seeds/community_deletion_checkpoints.sql
\i seeds/community_deletion_manifest_keys.sql
\i seeds/storage_taxonomy_sweeps.sql
\i seeds/community_serving_write_leases.sql
\i seeds/community_deletion_executor_heartbeats.sql
\i seeds/relay_operators.sql
\i seeds/relay_admin_actions.sql
\i seeds/relay_admin_outbox.sql
\i seeds/operator_listener_pubkeys.sql
\i seeds/operator_listener_outbox.sql
\i seeds/relay_operator_audit.sql
\i seeds/storage_accounting_snapshots.sql

-- Fence any community-scoped table that lacks an explicit attach.
\i community_write_fence.sql
