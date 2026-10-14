-- Push gateway desired-state database manifest.
--
-- The gateway's authority store is deployment-global and lives in its own
-- dedicated database, outside relay community tenancy. This directory declares
-- its end state; `../migrations/` remains the incremental path the gateway
-- binary applies at startup.
-- `src/postgres/desired_schema_postgres_tests.rs` proves the two produce the
-- same catalog.
--
-- `tables/public/<table>.sql` declares one table per file: its CREATE TABLE
-- and the CREATE INDEX statements on it, the one-table-per-file shape SchemaBot
-- reads. `tables/` is the SchemaBot schema directory and `public` is its
-- namespace, so nothing else may live under it. The gateway has no types,
-- functions, triggers, or seed rows. Its pre-migration bootstrap guard and
-- runtime-role grants are operational steps in `src/postgres.rs`, not schema.
-- It also has nothing pgschema cannot express (no seed DML, no storage
-- parameters), so the relay's `scripts/reconcile-schema-after-pgschema.sql`
-- step (root `AGENTS.md` gotcha #7) does not apply to it.
--
-- `./bin/pgschema apply --file crates/buzz-push-gateway/schema/schema.sql`
-- resolves the `\i` includes relative to this file. Raw `psql` resolves `\i`
-- against the working directory instead, so run it from here:
-- `cd crates/buzz-push-gateway/schema && psql -f schema.sql`.
--
-- Adding a table: add `tables/public/<table>.sql` and include it below after
-- the tables it references. This file holds only `\i` lines. The layout
-- test in `src/postgres/desired_schema_postgres_tests.rs` enforces both.

\i tables/public/push_gateway_challenges.sql
\i tables/public/push_gateway_installations.sql
\i tables/public/push_gateway_delegations.sql
\i tables/public/push_gateway_endpoint_quotas.sql
\i tables/public/push_gateway_delivery_auth_replays.sql
\i tables/public/push_gateway_delivery_request_replays.sql
