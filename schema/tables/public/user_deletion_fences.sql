-- Operator-global restrictions, deliberately retained after profile and community
-- deletion. scope_community_id is the scope of an agent restriction, not ownership
-- of this row. Never clear it during community deletion: NULL means a global user.
CREATE TABLE user_deletion_fences (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pubkey TEXT NOT NULL CHECK (pubkey ~ '^[0-9a-f]{64}$'),
    scope_community_id UUID,
    request_id UUID NOT NULL,
    source_owner_pubkey TEXT CHECK (source_owner_pubkey ~ '^[0-9a-f]{64}$'),
    blocked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((scope_community_id IS NULL) = (source_owner_pubkey IS NULL))
);

CREATE UNIQUE INDEX user_deletion_fences_global_key
    ON user_deletion_fences (pubkey) WHERE scope_community_id IS NULL;
CREATE UNIQUE INDEX user_deletion_fences_scoped_key
    ON user_deletion_fences (pubkey, scope_community_id) WHERE scope_community_id IS NOT NULL;
