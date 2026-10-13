-- NIP-CL state and atomic-application provenance. Supported application writers
-- validate canonical labels and serialize channel/metadata/evidence in one
-- admitted transaction. No new database function or trigger owns this protocol.
ALTER TABLE channels ADD COLUMN labels TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE channels ADD CONSTRAINT chk_channel_labels_count
    CHECK (cardinality(labels) <= 32);

-- The command row is its receipt only when the NIP-CL transaction marks it.
-- Legacy stored requests remain unknown. Soft deletion preserves provenance;
-- retention must not remove fresh commands (including future-dated commands).
ALTER TABLE events ADD COLUMN nip_cl_applied BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE events ADD CONSTRAINT chk_events_nip_cl_applied_kind
    CHECK (NOT nip_cl_applied OR kind IN (9002, 9007));
