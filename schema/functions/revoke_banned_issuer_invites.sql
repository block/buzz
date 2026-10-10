-- Revocation is durable: neither unban nor expiry restores an issued bearer.
CREATE FUNCTION revoke_banned_issuer_invites() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.banned AND (NEW.ban_expires_at IS NULL OR NEW.ban_expires_at > clock_timestamp()) THEN
        UPDATE relay_invites SET revoked_at = clock_timestamp()
        WHERE community_id = NEW.community_id AND revoked_at IS NULL
          AND (created_by = encode(NEW.pubkey, 'hex') OR created_by IN (
              SELECT encode(pubkey, 'hex') FROM users
              WHERE community_id = NEW.community_id AND agent_owner_pubkey = NEW.pubkey
          ));
    END IF;
    RETURN NEW;
END
$$;
