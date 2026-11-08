INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('entity_deletion_stage_history', 'Operator-global deletion workflow history retained for recovery and audit')
ON CONFLICT (table_name) DO NOTHING;
