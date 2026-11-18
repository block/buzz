INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('entity_deletion_approvals', 'Operator-global deletion approval history retained for audit')
ON CONFLICT (table_name) DO NOTHING;
