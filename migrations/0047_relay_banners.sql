-- Operator-configured deployment banners.
-- Exactly one active banner exists deployment-wide.  Scope is either all
-- communities (target_all_communities=true) or an explicit non-empty set in
-- relay_banner_communities.

CREATE TABLE relay_banners (
    id BIGSERIAL PRIMARY KEY,
    public_id UUID NOT NULL DEFAULT gen_random_uuid(),
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'urgent')),
    message TEXT NOT NULL CHECK (char_length(message) BETWEEN 1 AND 2000),
    max_displays INTEGER NOT NULL CHECK (max_displays >= 1),
    target_all_communities BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at TIMESTAMPTZ,
    created_by BYTEA NOT NULL CHECK (length(created_by) = 32),
    updated_by BYTEA NOT NULL CHECK (length(updated_by) = 32),
    disabled_by BYTEA CHECK (disabled_by IS NULL OR length(disabled_by) = 32)
);

CREATE UNIQUE INDEX idx_relay_banners_public_id
    ON relay_banners (public_id);

CREATE UNIQUE INDEX idx_relay_banners_one_active
    ON relay_banners ((true))
    WHERE disabled_at IS NULL;

CREATE INDEX idx_relay_banners_updated_at
    ON relay_banners (updated_at DESC, id DESC);

CREATE TABLE relay_banner_communities (
    banner_id BIGINT NOT NULL REFERENCES relay_banners(id) ON DELETE CASCADE,
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    PRIMARY KEY (banner_id, community_id)
);

CREATE INDEX idx_relay_banner_communities_community
    ON relay_banner_communities (community_id, banner_id);

CREATE TABLE relay_banner_user_state (
    banner_id BIGINT NOT NULL REFERENCES relay_banners(id) ON DELETE CASCADE,
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    pubkey BYTEA NOT NULL CHECK (length(pubkey) = 32),
    display_count INTEGER NOT NULL DEFAULT 0 CHECK (display_count >= 0),
    dismissed_at TIMESTAMPTZ,
    first_viewed_at TIMESTAMPTZ,
    last_viewed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (banner_id, community_id, pubkey),
    CHECK (first_viewed_at IS NULL OR last_viewed_at IS NOT NULL)
);

CREATE INDEX idx_relay_banner_user_state_pubkey
    ON relay_banner_user_state (community_id, pubkey, banner_id);

CREATE TABLE relay_banner_view_acks (
    banner_id BIGINT NOT NULL REFERENCES relay_banners(id) ON DELETE CASCADE,
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    pubkey BYTEA NOT NULL CHECK (length(pubkey) = 32),
    view_id UUID NOT NULL,
    viewed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (banner_id, community_id, pubkey, view_id)
);

CREATE INDEX idx_relay_banner_view_acks_pubkey
    ON relay_banner_view_acks (community_id, pubkey, banner_id);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('relay_banners', 'deployment-global operator-configured banner; no community_id intentionally'),
    ('relay_banner_communities', 'deployment-global banner targeting allowlist; community_id is target provenance only'),
    ('relay_banner_user_state', 'deployment-global per-user banner display state; community_id scopes the targeted impression only'),
    ('relay_banner_view_acks', 'deployment-global per-render banner idempotency keys; community_id scopes the targeted impression only');

SELECT attach_community_write_fence('relay_banner_communities');
SELECT attach_community_write_fence('relay_banner_user_state');
SELECT attach_community_write_fence('relay_banner_view_acks');
