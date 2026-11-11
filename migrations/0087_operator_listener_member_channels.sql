-- A registration's target-signed scope: false delivers only events that
-- p-tag the target; true also delivers channel events for channels the target
-- is a current member of. The constant default fills existing rows without a
-- table rewrite and keeps them mentions-only.
SET LOCAL lock_timeout = '5s';

ALTER TABLE operator_listener_pubkeys
    ADD COLUMN member_channels BOOLEAN NOT NULL DEFAULT false;

-- Member-channel enqueue starts from the few opted-in registrations and probes
-- channel_members by primary key, instead of walking every member of a busy
-- channel. Every existing row is false, so the index builds empty.
CREATE INDEX operator_listener_pubkeys_member_channels
    ON operator_listener_pubkeys (target_pubkey, listener_pubkey)
    WHERE member_channels;
