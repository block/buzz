-- Mirror of push gateway migration 0009 (docs/database.md Rule 4). The relay
-- never reads or writes the push_gateway_* tables: the gateway runs against its
-- own dedicated database. Adding this column to the relay's copy changes no
-- relay behavior. A constant default adds it without a table rewrite.
SET LOCAL lock_timeout = '5s';
ALTER TABLE push_gateway_delivery_request_replays
    ADD COLUMN finished BOOLEAN NOT NULL DEFAULT true;
