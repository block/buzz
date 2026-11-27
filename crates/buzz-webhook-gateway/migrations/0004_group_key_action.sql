SET LOCAL lock_timeout = '5s';

ALTER TABLE webhook_gateway_integrations
    ADD COLUMN group_key_action TEXT NOT NULL DEFAULT 'reply_in_thread',
    ADD CONSTRAINT webhook_gateway_integrations_group_key_action_check
        CHECK (group_key_action IN ('reply_in_thread', 'update_message')) NOT VALID;
