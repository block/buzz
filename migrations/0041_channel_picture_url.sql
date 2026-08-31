-- Persist the public channel-picture URL projected by relay-signed kind:39000
-- metadata. Protocol-level URL validation remains in the kind:9002 ingest
-- path; this storage guard keeps direct DB writers within the same coarse
-- HTTPS and size boundary while allowing NULL to clear the picture.
ALTER TABLE channels
    ADD COLUMN picture_url TEXT,
    ADD CONSTRAINT chk_channels_picture_url_safe
        CHECK (
            picture_url IS NULL
            OR (octet_length(picture_url) <= 2048 AND lower(picture_url) LIKE 'https://%')
        );
