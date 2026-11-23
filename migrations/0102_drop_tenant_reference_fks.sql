-- Contract step (docs/database.md Rule 2 and "Retiring a trigger, function,
-- or foreign key"): drop the 25 foreign keys between tenant tables. The
-- owning application transactions check each reference and delete or detach
-- dependent rows themselves.
--
-- Required stop: run a relay build with the application reference checks and
-- explicit child deletes (#9608 or later) on every pod and job first. Do not
-- roll back below it.
--
-- Dropping a foreign key takes ACCESS EXCLUSIVE on the child and on the
-- referenced table. Take every lock in one statement, children before
-- parents. The budgets end the attempt well before the 1s deadlock_timeout,
-- so this transaction fails instead of a relay query. Retry on failure.
SET LOCAL lock_timeout = '300ms';
SET LOCAL statement_timeout = '800ms';

LOCK TABLE
    join_policy_acceptances,
    personal_read_frontiers,
    push_wake_outbox,
    relay_admin_outbox,
    relay_admin_actions,
    moderation_reports,
    moderation_actions,
    thread_metadata,
    channel_members,
    scheduled_workflow_fires,
    workflow_approvals,
    workflow_runs,
    workflows,
    api_tokens,
    subscriptions,
    community_deletion_approvals,
    community_deletion_checkpoints,
    community_deletion_executor_heartbeats,
    community_deletion_manifest_keys,
    relay_members,
    personal_read_accounts,
    push_leases,
    channels,
    users,
    community_deletion_requests
    IN ACCESS EXCLUSIVE MODE;

ALTER TABLE join_policy_acceptances DROP CONSTRAINT join_policy_acceptances_community_id_pubkey_fkey;
ALTER TABLE personal_read_frontiers DROP CONSTRAINT personal_read_frontiers_community_id_actor_fkey;
ALTER TABLE personal_read_frontiers DROP CONSTRAINT personal_read_frontiers_community_id_channel_id_fkey;
ALTER TABLE push_wake_outbox DROP CONSTRAINT push_wake_outbox_community_id_author_installation_id_fkey;
ALTER TABLE relay_admin_outbox DROP CONSTRAINT relay_admin_outbox_action_id_fkey;
ALTER TABLE relay_admin_actions DROP CONSTRAINT relay_admin_actions_report_community_id_report_id_fkey;
ALTER TABLE moderation_reports DROP CONSTRAINT moderation_reports_community_id_action_id_fkey;
ALTER TABLE moderation_reports DROP CONSTRAINT moderation_reports_community_id_channel_id_fkey;
ALTER TABLE moderation_actions DROP CONSTRAINT moderation_actions_community_id_channel_id_fkey;
ALTER TABLE thread_metadata DROP CONSTRAINT thread_metadata_community_id_channel_id_fkey;
ALTER TABLE channel_members DROP CONSTRAINT channel_members_community_id_channel_id_fkey;
ALTER TABLE scheduled_workflow_fires DROP CONSTRAINT scheduled_workflow_fires_community_id_workflow_id_fkey;
ALTER TABLE scheduled_workflow_fires DROP CONSTRAINT scheduled_workflow_fires_community_id_workflow_run_id_fkey;
ALTER TABLE workflow_approvals DROP CONSTRAINT workflow_approvals_community_id_workflow_id_fkey;
ALTER TABLE workflow_approvals DROP CONSTRAINT workflow_approvals_community_id_run_id_fkey;
ALTER TABLE workflow_runs DROP CONSTRAINT workflow_runs_community_id_workflow_id_fkey;
ALTER TABLE workflows DROP CONSTRAINT workflows_community_id_owner_pubkey_fkey;
ALTER TABLE workflows DROP CONSTRAINT workflows_community_id_channel_id_fkey;
ALTER TABLE api_tokens DROP CONSTRAINT api_tokens_community_id_owner_pubkey_fkey;
ALTER TABLE subscriptions DROP CONSTRAINT subscriptions_community_id_owner_pubkey_fkey;
ALTER TABLE users DROP CONSTRAINT users_community_id_agent_owner_pubkey_fkey;
ALTER TABLE community_deletion_approvals DROP CONSTRAINT community_deletion_approvals_request_id_community_id_inven_fkey;
ALTER TABLE community_deletion_checkpoints DROP CONSTRAINT community_deletion_checkpoints_request_id_fkey;
ALTER TABLE community_deletion_executor_heartbeats DROP CONSTRAINT community_deletion_executor_heartbeats_request_id_fkey;
ALTER TABLE community_deletion_manifest_keys DROP CONSTRAINT community_deletion_manifest_keys_request_id_fkey;
