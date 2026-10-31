-- System channels: channels that exist only for access control, for example
-- to hold config that only an agent and its owner can read. They are ordinary
-- channels in every other way. The relay leaves them out of queries unless a
-- filter names the channel or asks for system channels with #t:["system"]
-- on kinds 39000-39003.
--
-- Additive. Do not roll the relay back past this change while system
-- channels exist: an older relay cannot read the value and does not exclude
-- these channels.
ALTER TYPE channel_type ADD VALUE IF NOT EXISTS 'system';
