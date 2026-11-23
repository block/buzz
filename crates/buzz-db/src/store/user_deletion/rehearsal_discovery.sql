-- Submission only: discover candidate communities, never authorize row deletion.
-- Include inactive agents and removed memberships. A current owned-agent profile
-- already selects its community; do not carry its key into another community.
-- Keep all predicates inside the limit so unrelated tenants cannot exhaust it.
SELECT c.id AS community_id, c.host, m.role
FROM communities c
LEFT JOIN relay_members m ON m.community_id = c.id AND m.pubkey = $1
WHERE m.pubkey IS NOT NULL
OR EXISTS (SELECT 1 FROM users WHERE community_id = c.id
    AND (pubkey = decode($1, 'hex') OR agent_owner_pubkey = decode($1, 'hex')))
OR EXISTS (SELECT 1 FROM channel_members WHERE community_id = c.id
    AND (pubkey = decode($1, 'hex') OR invited_by = decode($1, 'hex') OR removed_by = decode($1, 'hex')))
OR EXISTS (SELECT 1 FROM relay_members WHERE community_id = c.id AND added_by = $1)
OR EXISTS (SELECT 1 FROM api_tokens WHERE community_id = c.id
    AND (owner_pubkey = decode($1, 'hex') OR revoked_by = decode($1, 'hex')))
OR EXISTS (SELECT 1 FROM relay_invites WHERE community_id = c.id AND created_by = $1)
OR EXISTS (SELECT 1 FROM product_feedback WHERE community_id = c.id AND submitter_pubkey = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM join_policy_acceptances WHERE community_id = c.id AND pubkey = $1)
OR EXISTS (SELECT 1 FROM pubkey_allowlist WHERE community_id = c.id
    AND (pubkey = decode($1, 'hex') OR added_by = decode($1, 'hex')))
OR EXISTS (SELECT 1 FROM personal_read_frontiers WHERE community_id = c.id AND actor = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM personal_read_accounts WHERE community_id = c.id AND actor = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM nip_fi_profile_heads WHERE community_id = c.id AND pubkey = $1)
OR EXISTS (SELECT 1 FROM archived_identities WHERE community_id = c.id AND (pubkey = $1 OR actor = $1 OR replaced_by = $1))
OR EXISTS (SELECT 1 FROM reactions WHERE community_id = c.id AND pubkey = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM push_leases WHERE community_id = c.id AND author = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM push_wake_outbox WHERE community_id = c.id AND author = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM operator_listener_outbox WHERE community_id = c.id
    AND (target_pubkey = decode($1, 'hex') OR listener_pubkey = decode($1, 'hex')))
OR EXISTS (SELECT 1 FROM media_uploaders WHERE community_id = c.id AND pubkey = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM audit_log WHERE community_id = c.id
    AND action = 'media_uploaded' AND actor_pubkey = decode($1, 'hex'))
OR EXISTS (SELECT 1 FROM workflow_runs WHERE community_id = c.id AND cause_pubkey = decode($1, 'hex')
    AND jsonb_typeof(trigger_context) = 'object'
    AND ((cause_source = 'event' AND jsonb_typeof(trigger_context->'text') = 'string'
        AND jsonb_typeof(trigger_context->'emoji') = 'string'
        AND (trigger_context->>'text' <> '' OR trigger_context->>'emoji' <> ''))
      OR (cause_source = 'manual' AND jsonb_typeof(trigger_context->'webhook_fields') = 'object'
        AND trigger_context->'webhook_fields' <> '{}'::jsonb)))
OR EXISTS (SELECT 1 FROM workflow_approvals WHERE community_id = c.id
    AND approver_pubkey = decode($1, 'hex') AND note IS NOT NULL)
-- A retained owned workflow is a profile-cleanup prerequisite too.
OR EXISTS (SELECT 1 FROM workflows WHERE community_id = c.id AND owner_pubkey = decode($1, 'hex'))
-- Reuse the content handler's exact signer/actor and private-state attribution.
OR EXISTS (SELECT 1 FROM (
    /* ATTRIBUTED_CONTENT */
) attributed WHERE pubkey = decode($3, 'hex') AND kind NOT IN (30617,30618))
OR EXISTS (
    SELECT 1 FROM events e WHERE e.community_id = c.id AND (
        -- Profile prerequisites include unsupported authored content, but
        -- repository announcements/state are explicitly retained without blocking.
        (e.pubkey = decode($1, 'hex') AND e.kind NOT IN (30617,30618))
        -- Profile handlers must inspect subject/replacement candidates even if
        -- malformed: silently skipping them would hide a cleanup blocker.
        OR (e.kind IN (30177,30179,30382,9035,9036,8002,8003,13535)
            AND EXISTS (SELECT 1 FROM jsonb_array_elements(
                CASE WHEN jsonb_typeof(e.tags) = 'array' THEN e.tags ELSE '[]'::jsonb END) tag
                WHERE (tag->>0 IN ('p','d') AND lower(tag->>1) = $1)
                  OR (tag->>0 = 'consent' AND lower(tag->>2) = $1)
                  OR (e.kind IN (9035,9036,8002,8003) AND tag->>0 = 'replaced-by' AND lower(tag->>1) = $1)))
    )
)
ORDER BY c.id LIMIT $2
