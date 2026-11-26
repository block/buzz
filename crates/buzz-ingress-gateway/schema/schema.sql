-- Ingress gateway desired-state database manifest.
--
-- The gateway's store is deployment-global and lives in its own dedicated
-- database, outside relay community tenancy. This directory declares its end
-- state; `../migrations/` is the incremental path the gateway binary applies
-- at startup. `tables/public/<table>.sql` declares one table per file: its
-- CREATE TABLE and the CREATE INDEX statements on it. The gateway has no
-- types, functions, triggers, foreign keys, or seed rows (docs/database.md).
\i tables/public/ingress_gateway_integrations.sql
\i tables/public/ingress_gateway_integration_secrets.sql
\i tables/public/ingress_gateway_applied_operations.sql
\i tables/public/ingress_gateway_deliveries.sql
\i tables/public/ingress_gateway_subscriptions.sql
\i tables/public/ingress_gateway_messages.sql
