-- Contract step (docs/database.md Rule 2): drop the community_id foreign keys
-- on the 32 unpartitioned tenant tables. Writers prove the community inside
-- their own transaction (AdmittedTx, or an INSERT ... SELECT from an active
-- community), and community deletion purges children explicitly before it
-- tombstones the community row. Production code never deletes a community.
--
-- Required stop: as for 0102.
--
-- Each drop takes ACCESS EXCLUSIVE on the child and on communities. Take
-- every lock in one statement, communities last. The budgets end the attempt
-- well before the 1s deadlock_timeout, so this transaction fails instead of a
-- relay query. Retry on failure.
SET LOCAL lock_timeout = '300ms';
SET LOCAL statement_timeout = '800ms';

LOCK TABLE
    api_tokens,
    archived_identities,
    artifact_heads,
    artifact_revisions,
    audit_log,
    channel_members,
    channels,
    community_bans,
    community_deletion_requests,
    community_nip_fi_issuers,
    community_serving_write_leases,
    event_mentions,
    git_repo_names,
    moderation_actions,
    moderation_reports,
    parameterized_event_watermarks,
    personal_read_accounts,
    product_feedback,
    pubkey_allowlist,
    push_leases,
    push_match_queue,
    push_wake_outbox,
    reactions,
    relay_invites,
    relay_members,
    scheduled_workflow_fires,
    subscriptions,
    thread_metadata,
    users,
    workflow_approvals,
    workflow_runs,
    workflows,
    communities
    IN ACCESS EXCLUSIVE MODE;

ALTER TABLE api_tokens DROP CONSTRAINT api_tokens_community_id_fkey;
ALTER TABLE archived_identities DROP CONSTRAINT archived_identities_community_id_fkey;
ALTER TABLE artifact_heads DROP CONSTRAINT artifact_heads_community_id_fkey;
ALTER TABLE artifact_revisions DROP CONSTRAINT artifact_revisions_community_id_fkey;
ALTER TABLE audit_log DROP CONSTRAINT audit_log_community_id_fkey;
ALTER TABLE channel_members DROP CONSTRAINT channel_members_community_id_fkey;
ALTER TABLE channels DROP CONSTRAINT channels_community_id_fkey;
ALTER TABLE community_bans DROP CONSTRAINT community_bans_community_id_fkey;
ALTER TABLE community_deletion_requests DROP CONSTRAINT community_deletion_requests_community_id_fkey;
ALTER TABLE community_nip_fi_issuers DROP CONSTRAINT community_nip_fi_issuers_community_id_fkey;
ALTER TABLE community_serving_write_leases DROP CONSTRAINT community_serving_write_leases_community_id_fkey;
ALTER TABLE event_mentions DROP CONSTRAINT event_mentions_community_id_fkey;
ALTER TABLE git_repo_names DROP CONSTRAINT git_repo_names_community_id_fkey;
ALTER TABLE moderation_actions DROP CONSTRAINT moderation_actions_community_id_fkey;
ALTER TABLE moderation_reports DROP CONSTRAINT moderation_reports_community_id_fkey;
ALTER TABLE parameterized_event_watermarks DROP CONSTRAINT parameterized_event_watermarks_community_id_fkey;
ALTER TABLE personal_read_accounts DROP CONSTRAINT personal_read_accounts_community_id_fkey;
ALTER TABLE product_feedback DROP CONSTRAINT product_feedback_community_id_fkey;
ALTER TABLE pubkey_allowlist DROP CONSTRAINT pubkey_allowlist_community_id_fkey;
ALTER TABLE push_leases DROP CONSTRAINT push_leases_community_id_fkey;
ALTER TABLE push_match_queue DROP CONSTRAINT push_match_queue_community_id_fkey;
ALTER TABLE push_wake_outbox DROP CONSTRAINT push_wake_outbox_community_id_fkey;
ALTER TABLE reactions DROP CONSTRAINT reactions_community_id_fkey;
ALTER TABLE relay_invites DROP CONSTRAINT relay_invites_community_id_fkey;
ALTER TABLE relay_members DROP CONSTRAINT relay_members_community_id_fkey;
ALTER TABLE scheduled_workflow_fires DROP CONSTRAINT scheduled_workflow_fires_community_id_fkey;
ALTER TABLE subscriptions DROP CONSTRAINT subscriptions_community_id_fkey;
ALTER TABLE thread_metadata DROP CONSTRAINT thread_metadata_community_id_fkey;
ALTER TABLE users DROP CONSTRAINT users_community_id_fkey;
ALTER TABLE workflow_approvals DROP CONSTRAINT workflow_approvals_community_id_fkey;
ALTER TABLE workflow_runs DROP CONSTRAINT workflow_runs_community_id_fkey;
ALTER TABLE workflows DROP CONSTRAINT workflows_community_id_fkey;
