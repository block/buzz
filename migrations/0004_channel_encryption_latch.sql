-- Add the E2E encryption activation latch to the channels table.
--
-- `encryption_activated_at` marks the point from which messages in this channel
-- MUST be NIP-44 v2 ciphertext. It is the tamper-evident encryption-start
-- marker for hybrid E2E: rather than a free-standing client event (which the
-- relay would have to locate per message via a sibling read, and which a member
-- could backdate), the boundary is relay-owned channel state. The relay sets it
-- and never lets a client write it, so it cannot be forged or moved.
--
-- Phase 1 sets this at DM creation (every message in a new DM is E2E). Existing
-- plaintext DMs leave it NULL and stay readable as legacy plaintext — no
-- re-encryption, which would change event IDs and break the audit hash-chain.
ALTER TABLE channels
    ADD COLUMN encryption_activated_at TIMESTAMPTZ;
