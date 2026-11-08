-- ── Moderation audit ──────────────────────────────────────────────────────────
-- One row per accepted moderation action. Full detail (reporter identities,
-- private reasons, matched NIP-OA principal) stays mod/audit-only; the public
-- tombstone carries only action_id + reason_code + sanitized public_reason.

CREATE TABLE moderation_actions (
    community_id    UUID NOT NULL REFERENCES communities(id),
    id              UUID NOT NULL DEFAULT gen_random_uuid(),
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    action          TEXT NOT NULL CHECK (action IN (
                        'delete_message', 'kick', 'ban', 'unban',
                        'timeout', 'untimeout', 'dismiss_report', 'escalate',
                        'resolve:delete', 'resolve:kick', 'resolve:ban',
                        'resolve:timeout')),
    target_pubkey   BYTEA CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32),
    target_event_id BYTEA CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    channel_id      UUID,
    -- Machine-readable rule/reason code (e.g. "spam", "community_rule_3").
    reason_code     TEXT,
    -- Sanitized, safe for the public tombstone.
    public_reason   TEXT,
    -- Mod-only context; never leaves the audit surface.
    private_reason  TEXT,
    -- NIP-OA: which principal matched a ban ('self' | 'owner'); audit-only,
    -- the client never learns which.
    matched_principal TEXT CHECK (matched_principal IS NULL OR matched_principal IN ('self', 'owner')),
    -- Deployment authority type for HTTP-initiated actions.
    actor_authority   TEXT NOT NULL DEFAULT 'community'
                      CHECK (actor_authority IN ('community', 'relay_operator', 'relay_moderator')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id)
);

CREATE INDEX idx_moderation_actions_created
    ON moderation_actions (community_id, created_at DESC);
CREATE INDEX idx_moderation_actions_target_pubkey
    ON moderation_actions (community_id, target_pubkey)
    WHERE target_pubkey IS NOT NULL;
