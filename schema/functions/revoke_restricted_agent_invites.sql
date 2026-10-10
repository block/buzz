-- Learning a verified owner must not revive invites issued by its restricted agent.
CREATE FUNCTION revoke_restricted_agent_invites() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.agent_owner_pubkey IS NOT NULL AND EXISTS (
        SELECT 1 FROM community_bans WHERE community_id = NEW.community_id
          AND pubkey = NEW.agent_owner_pubkey AND banned
          AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())
    ) THEN
        UPDATE relay_invites SET revoked_at = clock_timestamp()
        WHERE community_id = NEW.community_id AND created_by = encode(NEW.pubkey, 'hex')
          AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END
$$;
