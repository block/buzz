-- Each installation records the platform whose push service delivers its wake.
-- Expand only: a constant default keeps the deployed gateway's inserts valid
-- (every installation it writes is iOS) and marks every existing row iOS.
-- Bound the ACCESS EXCLUSIVE lock wait and the CHECK constraint's validation scan.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE push_gateway_installations
    ADD COLUMN platform TEXT NOT NULL DEFAULT 'ios' CHECK (platform IN ('ios', 'android'));
