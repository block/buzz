-- Channel labels: creator- or admin-chosen tags that group channels across
-- channel types. The relay publishes each label as a ["t", <label>] tag on
-- the channel's group-state events (kinds 39000-39003), after the channel
-- type tag. kind:9007 sets them and kind:9002 replaces them; the relay
-- validates their shape (1-64 of a-z 0-9 . : -, not a channel type name).
--
-- Additive: existing channels get an empty set and look as before. The
-- CHECK bounds the set even if a writer skips the relay's validation.
-- No index yet: label lookups match the GIN-indexed tags of the relay-signed
-- group-state events, not this column.
ALTER TABLE channels
    ADD COLUMN labels TEXT[] NOT NULL DEFAULT '{}'
        CONSTRAINT channels_labels_bounded CHECK (cardinality(labels) <= 8);
