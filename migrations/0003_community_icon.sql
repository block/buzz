-- ── Per-community workspace icon (NIP-11 `icon`) ─────────────────────────────
-- Set by relay admins/owners via the kind:9033 command; served to clients in
-- the standard NIP-11 relay information document `icon` field, bound to the
-- community resolved from the request Host (same row-zero seam as WS/NIP-05).
--
-- Additive migration: previously applied files must not change checksum.
-- `communities` is an operator-global registry table (no community_id column
-- by design); this adds a per-row presentation attribute, not tenant data in
-- a shared table. TEXT holds either an http(s) URL or a small data:image/*
-- URL — validated and size-capped at the 9033 write path, not here.

ALTER TABLE communities ADD COLUMN icon TEXT;
