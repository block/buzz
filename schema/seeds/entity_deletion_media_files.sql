INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('entity_deletion_media_files', 'Operator-global media inventory and recovery evidence; retained after community deletion')
ON CONFLICT (table_name) DO NOTHING;
