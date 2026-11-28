/**
 * Client mirror of the relay's kind:9001 authority for removing a channel
 * member (`classify_remove_other` in `buzz-relay` `channel_authz.rs`):
 *
 * - Anyone may remove themselves. The relay separately keeps the last owner.
 * - Only an owner may remove an owner, even an owner that is the actor's own
 *   agent.
 * - Admins may remove anyone else; owners may remove anyone but another owner.
 * - Any other member may remove only their own agents.
 */
export function canRemoveChannelMember({
  memberPubkey,
  memberRole,
  currentPubkey,
  selfRole,
  isOwnAgent,
}: {
  memberPubkey: string;
  memberRole: string;
  currentPubkey: string | undefined;
  selfRole: string | null | undefined;
  isOwnAgent: boolean;
}): boolean {
  if (memberPubkey === currentPubkey) {
    return true;
  }
  if (memberRole === "owner" && selfRole !== "owner") {
    return false;
  }
  return (
    selfRole === "admin" ||
    (selfRole === "owner" && memberRole !== "owner") ||
    (selfRole != null && isOwnAgent)
  );
}
