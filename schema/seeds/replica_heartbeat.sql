INSERT INTO replica_heartbeat (id) VALUES (1);
INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('replica_heartbeat', 'single-row replication freshness token; describes deployment topology, never tenant data');
