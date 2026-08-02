-- #4173: record, per audit_log row, which hash-encoding version its digest
-- was computed with. The legacy (v1) preimage concatenated variable-length
-- fields with no framing, so two distinct entries could share a digest
-- ((object_id 'x', detail 12) vs (object_id 'x1', detail 2)) and a
-- boundary-shifting edit could survive verify_chain. New entries are hashed
-- with the length-prefixed v2 encoding; verification recomputes each row
-- under its stored version, so existing chains keep verifying unchanged.
--
-- DEFAULT 1 is deliberate and must outlive this release: during a rolling
-- deploy, pods that predate the column still INSERT without naming it, and
-- their rows are genuinely v1-hashed. The relay's write path stamps 2
-- explicitly; dropping the DEFAULT is a later cleanup once no pre-upgrade
-- writers remain.
ALTER TABLE audit_log
    ADD COLUMN hash_version SMALLINT NOT NULL DEFAULT 1;
