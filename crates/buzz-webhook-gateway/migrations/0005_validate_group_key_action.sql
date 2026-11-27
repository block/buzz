SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- Validate after the column-add transaction releases its exclusive lock.
ALTER TABLE webhook_gateway_integrations
    VALIDATE CONSTRAINT webhook_gateway_integrations_group_key_action_check;
