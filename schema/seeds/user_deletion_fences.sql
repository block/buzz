INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('user_deletion_fences', 'Operator-global user and agent restrictions retained after profile and community deletion')
ON CONFLICT (table_name) DO NOTHING;
