-- Canonical public community name. Existing communities remain unnamed until
-- an owner/admin explicitly publishes a name; never promote a device label.
ALTER TABLE communities ADD COLUMN name TEXT;
ALTER TABLE communities ADD CONSTRAINT communities_name_valid CHECK (
    name IS NULL OR (octet_length(name) BETWEEN 1 AND 256 AND name = btrim(name))
);
