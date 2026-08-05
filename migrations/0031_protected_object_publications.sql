-- PostgreSQL-authoritative visibility for protected object-store content.
-- Immutable objects may be staged before these rows commit; without a current
-- publication row they are not visible in Enforce.

CREATE TABLE git_repo_publications (
    community_id       UUID NOT NULL,
    repo_id            TEXT NOT NULL,
    owner_pubkey       TEXT NOT NULL,
    manifest_sha256    TEXT NOT NULL CHECK (
        manifest_sha256 ~ '^[0-9a-f]{64}$'
    ),
    publication_version BIGINT NOT NULL CHECK (publication_version > 0),
    state              TEXT NOT NULL DEFAULT 'active' CHECK (
        state IN ('active', 'unpublished')
    ),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, repo_id),
    FOREIGN KEY (community_id, repo_id)
        REFERENCES git_repo_names (community_id, repo_id)
);

CREATE TABLE media_publications (
    community_id       UUID NOT NULL REFERENCES communities(id),
    sha256              TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    object_key          TEXT NOT NULL CHECK (
        length(object_key) > 0 AND length(object_key) <= 512
    ),
    extension           TEXT NOT NULL CHECK (extension ~ '^[a-z0-9]{1,8}$'),
    mime_type           TEXT NOT NULL CHECK (
        length(mime_type) > 0 AND length(mime_type) <= 255
    ),
    object_size         BIGINT NOT NULL CHECK (object_size >= 0),
    metadata            JSONB NOT NULL CHECK (
        octet_length(metadata::text) <= 16384
    ),
    thumbnail_key       TEXT CHECK (
        thumbnail_key IS NULL OR (length(thumbnail_key) > 0 AND length(thumbnail_key) <= 512)
    ),
    publication_version BIGINT NOT NULL DEFAULT 1 CHECK (publication_version > 0),
    state               TEXT NOT NULL DEFAULT 'active' CHECK (
        state IN ('active', 'unpublished')
    ),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, sha256)
);

CREATE INDEX idx_media_publications_state
    ON media_publications (community_id, state);
