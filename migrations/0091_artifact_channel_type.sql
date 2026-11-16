-- Artifact conversations: a channel that holds an artifact's comments, such as
-- a task's. Its ID is derived from the artifact, its parent is the artifact's
-- home, and access comes only through that parent. Like 'system', it is not a
-- legacy type, so the relay leaves it out of queries that do not name it.
--
-- Additive. Do not roll the relay back past this change while artifact
-- conversations exist: an older relay cannot read the value and does not
-- exclude these channels.
ALTER TYPE channel_type ADD VALUE IF NOT EXISTS 'artifact';
