INSERT INTO replica_heartbeat (id) VALUES (1) ON CONFLICT (id) DO NOTHING;
INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('replica_heartbeat', 'single-row replication freshness token; describes deployment topology, never tenant data')
ON CONFLICT (table_name) DO NOTHING;
