SELECT attach_community_write_fence('community_bans');

CREATE TRIGGER invite_restriction_lock BEFORE INSERT OR UPDATE ON community_bans
FOR EACH ROW EXECUTE FUNCTION lock_invite_restrictions();
CREATE TRIGGER invite_issuer_revocation AFTER INSERT OR UPDATE ON community_bans
FOR EACH ROW EXECUTE FUNCTION revoke_banned_issuer_invites();
