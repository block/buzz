-- Directly provisioned observers are relay members whose transport authority
-- is restricted by the shared principal resolver to channel-event reads.
ALTER TABLE relay_members
    DROP CONSTRAINT relay_members_role_check;

ALTER TABLE relay_members
    ADD CONSTRAINT relay_members_role_check
    CHECK (role IN ('owner', 'admin', 'member', 'observer'));
