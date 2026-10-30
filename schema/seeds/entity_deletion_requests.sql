INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('entity_deletion_requests', 'Operator-global synthetic user-deletion intake')
ON CONFLICT (table_name) DO NOTHING;
