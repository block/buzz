SELECT attach_community_write_fence('users');

CREATE TRIGGER invite_owner_lock BEFORE INSERT OR UPDATE OF agent_owner_pubkey ON users
FOR EACH ROW EXECUTE FUNCTION lock_invite_restrictions();
CREATE TRIGGER invite_owner_revocation AFTER INSERT OR UPDATE OF agent_owner_pubkey ON users
FOR EACH ROW EXECUTE FUNCTION revoke_restricted_agent_invites();
