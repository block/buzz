SET LOCAL lock_timeout = '5s';

-- The fleet has moved to app-side admission for all supported serving writers.
-- Drop only the universal community write-fence triggers. The community
-- tombstone and all other table guards remain in place.
DROP TRIGGER IF EXISTS community_write_fence_api_tokens ON api_tokens;
DROP TRIGGER IF EXISTS community_write_fence_archived_identities ON archived_identities;
DROP TRIGGER IF EXISTS community_write_fence_artifact_heads ON artifact_heads;
DROP TRIGGER IF EXISTS community_write_fence_artifact_revisions ON artifact_revisions;
DROP TRIGGER IF EXISTS community_write_fence_audit_log ON audit_log;
DROP TRIGGER IF EXISTS community_write_fence_channel_members ON channel_members;
DROP TRIGGER IF EXISTS community_write_fence_channels ON channels;
DROP TRIGGER IF EXISTS community_write_fence_community_bans ON community_bans;
DROP TRIGGER IF EXISTS community_write_fence_delivery_log ON delivery_log;
DROP TRIGGER IF EXISTS community_write_fence_event_mentions ON event_mentions;
DROP TRIGGER IF EXISTS community_write_fence_events ON events;
DROP TRIGGER IF EXISTS community_write_fence_git_repo_names ON git_repo_names;
DROP TRIGGER IF EXISTS community_write_fence_join_policy_acceptances ON join_policy_acceptances;
DROP TRIGGER IF EXISTS community_write_fence_moderation_actions ON moderation_actions;
DROP TRIGGER IF EXISTS community_write_fence_moderation_reports ON moderation_reports;
DROP TRIGGER IF EXISTS community_write_fence_parameterized_event_watermarks ON parameterized_event_watermarks;
DROP TRIGGER IF EXISTS community_write_fence_personal_read_accounts ON personal_read_accounts;
DROP TRIGGER IF EXISTS community_write_fence_personal_read_frontiers ON personal_read_frontiers;
DROP TRIGGER IF EXISTS community_write_fence_pubkey_allowlist ON pubkey_allowlist;
DROP TRIGGER IF EXISTS community_write_fence_push_leases ON push_leases;
DROP TRIGGER IF EXISTS community_write_fence_push_match_queue ON push_match_queue;
DROP TRIGGER IF EXISTS community_write_fence_push_wake_outbox ON push_wake_outbox;
DROP TRIGGER IF EXISTS community_write_fence_reactions ON reactions;
DROP TRIGGER IF EXISTS community_write_fence_relay_invites ON relay_invites;
DROP TRIGGER IF EXISTS community_write_fence_relay_members ON relay_members;
DROP TRIGGER IF EXISTS community_write_fence_scheduled_workflow_fires ON scheduled_workflow_fires;
DROP TRIGGER IF EXISTS community_write_fence_subscriptions ON subscriptions;
DROP TRIGGER IF EXISTS community_write_fence_thread_metadata ON thread_metadata;
DROP TRIGGER IF EXISTS community_write_fence_users ON users;
DROP TRIGGER IF EXISTS community_write_fence_workflow_approvals ON workflow_approvals;
DROP TRIGGER IF EXISTS community_write_fence_workflow_runs ON workflow_runs;
DROP TRIGGER IF EXISTS community_write_fence_workflows ON workflows;

DROP FUNCTION attach_community_write_fence(REGCLASS);
DROP FUNCTION enforce_community_write_fence();
DROP FUNCTION assert_community_write_allowed(UUID);
DROP FUNCTION community_write_allowed(UUID);
DROP FUNCTION community_write_fence_excluded_table(NAME);
