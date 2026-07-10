-- Hard community deletion is intentionally implemented at the ownership root.
-- Every tenant-scoped row has a direct community foreign key, so cascading
-- these 22 parent-table constraints makes one DELETE atomic and complete.
-- PostgreSQL propagates the events/delivery_log parent constraint changes to
-- their partitions; partition constraint clones must not be altered directly.

ALTER TABLE api_tokens
    DROP CONSTRAINT api_tokens_community_id_fkey,
    ADD CONSTRAINT api_tokens_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE archived_identities
    DROP CONSTRAINT archived_identities_community_id_fkey,
    ADD CONSTRAINT archived_identities_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE audit_log
    DROP CONSTRAINT audit_log_community_id_fkey,
    ADD CONSTRAINT audit_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE channel_members
    DROP CONSTRAINT channel_members_community_id_fkey,
    ADD CONSTRAINT channel_members_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE channels
    DROP CONSTRAINT channels_community_id_fkey,
    ADD CONSTRAINT channels_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE community_bans
    DROP CONSTRAINT community_bans_community_id_fkey,
    ADD CONSTRAINT community_bans_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE delivery_log
    DROP CONSTRAINT delivery_log_community_id_fkey,
    ADD CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE event_mentions
    DROP CONSTRAINT event_mentions_community_id_fkey,
    ADD CONSTRAINT event_mentions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE events
    DROP CONSTRAINT events_community_id_fkey,
    ADD CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE git_repo_names
    DROP CONSTRAINT git_repo_names_community_id_fkey,
    ADD CONSTRAINT git_repo_names_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE moderation_actions
    DROP CONSTRAINT moderation_actions_community_id_fkey,
    ADD CONSTRAINT moderation_actions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE moderation_reports
    DROP CONSTRAINT moderation_reports_community_id_fkey,
    ADD CONSTRAINT moderation_reports_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE pubkey_allowlist
    DROP CONSTRAINT pubkey_allowlist_community_id_fkey,
    ADD CONSTRAINT pubkey_allowlist_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE reactions
    DROP CONSTRAINT reactions_community_id_fkey,
    ADD CONSTRAINT reactions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE relay_members
    DROP CONSTRAINT relay_members_community_id_fkey,
    ADD CONSTRAINT relay_members_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE scheduled_workflow_fires
    DROP CONSTRAINT scheduled_workflow_fires_community_id_fkey,
    ADD CONSTRAINT scheduled_workflow_fires_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE subscriptions
    DROP CONSTRAINT subscriptions_community_id_fkey,
    ADD CONSTRAINT subscriptions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE thread_metadata
    DROP CONSTRAINT thread_metadata_community_id_fkey,
    ADD CONSTRAINT thread_metadata_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE users
    DROP CONSTRAINT users_community_id_fkey,
    ADD CONSTRAINT users_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE workflow_approvals
    DROP CONSTRAINT workflow_approvals_community_id_fkey,
    ADD CONSTRAINT workflow_approvals_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE workflow_runs
    DROP CONSTRAINT workflow_runs_community_id_fkey,
    ADD CONSTRAINT workflow_runs_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE workflows
    DROP CONSTRAINT workflows_community_id_fkey,
    ADD CONSTRAINT workflows_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
