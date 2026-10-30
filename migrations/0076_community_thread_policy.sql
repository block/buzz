-- ── Per-community thread policy (NIP-TP) ─────────────────────────────────────
-- Set by relay admins/owners via the kind:9034 command; served to clients in
-- the NIP-11 `thread_policy` field, bound to the community resolved from the
-- request Host (same row-zero seam as the 0003 workspace icon).
--
-- `flat` is a read-time presentation rule: stored thread_metadata keeps each
-- reply's true parent and depth, so switching back to `nested` restores the
-- original tree. Existing rows take the NIP-TP default, `nested`; the constant
-- default needs no backfill. `communities` is the small operator-global
-- registry, so validating the inline CHECK is cheap.
SET LOCAL lock_timeout = '5s';

ALTER TABLE communities
    ADD COLUMN thread_policy TEXT NOT NULL DEFAULT 'nested'
        CHECK (thread_policy IN ('nested', 'flat'));
