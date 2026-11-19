-- Mirror of push gateway migration 0008 (docs/database.md Rule 4). The relay
-- never reads or writes the push_gateway_* tables: the gateway runs against its
-- own dedicated database. Dropping this copy of the foreign key changes no
-- relay behavior.
SET LOCAL lock_timeout = '5s';
ALTER TABLE push_gateway_delegations
    DROP CONSTRAINT IF EXISTS push_gateway_delegations_installation_id_fkey;
