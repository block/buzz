-- Each installation records the platform whose push service delivers its wake.
-- Expand only: a constant default keeps the deployed gateway's inserts valid
-- (every installation it writes is iOS) and marks every existing row iOS.
ALTER TABLE push_gateway_installations
    ADD COLUMN platform TEXT NOT NULL DEFAULT 'ios' CHECK (platform IN ('ios', 'android'));
