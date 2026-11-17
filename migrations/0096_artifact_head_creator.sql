-- Artifact creator and assignees on the head (NIP-AR, Assignees and
-- creator). The head outlives event retention and redaction, so the relay
-- checks later revisions against it rather than against earlier payloads.
--
-- `creator` is the pubkey a `creator` tag named on create; it never changes.
-- `assignees` is the current revision's assignees, so a revision is checked
-- only for the people it adds.
--
-- Both are NULL for artifacts saved before this change: not yet known, which
-- is distinct from an empty assignee list. The relay fills them on the
-- artifact's next save, inside its lock (migrations carry no backfills).
--
-- Additive and nullable, so no rewrite, under bounded locks.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

ALTER TABLE artifact_heads ADD COLUMN IF NOT EXISTS creator TEXT;
ALTER TABLE artifact_heads ADD COLUMN IF NOT EXISTS assignees TEXT[];
