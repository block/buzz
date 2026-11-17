-- Narrow, opt-in contribution grants; existing channels remain unchanged.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE channels
    ADD COLUMN guest_replies BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN guest_reactions BOOLEAN NOT NULL DEFAULT false;
