INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('scheduled_message_disconnects', 'Issuer-wide disconnects fence deferred actions across communities')
ON CONFLICT (table_name) DO NOTHING;
