-- Artifact creator: the pubkey a `creator` tag named on create (NIP-AR,
-- Assignees and creator). It never changes, and the relay checks every
-- later revision against it, so it lives on the head, which outlives event
-- retention and redaction. NULL when the artifact was created without one,
-- including every artifact from before this change.
--
-- Additive and nullable, so no rewrite and no backfill, under bounded locks.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

ALTER TABLE artifact_heads ADD COLUMN IF NOT EXISTS creator TEXT;
