export interface RemovableSelf {
  role?: string | null;
}

export interface RemovableMember {
  role?: string | null;
  pubkey: string;
}

/**
 * Whether the acting member (`selfMember`) may remove `member` from the channel.
 *
 * Policy (must mirror the backend authorization in
 * `crates/buzz-relay/src/handlers/side_effects.rs`):
 * - An admin may remove any other member (not self).
 * - An owner may remove any other member, INCLUDING another owner (not self).
 *   The backend rejects removing the last remaining owner and the UI surfaces
 *   that error, so the client does not need to re-check the owner count here.
 * - Any member may remove a bot they own.
 * - Any member may remove themselves (self-removal), regardless of role.
 */
export function canRemoveMember<M extends RemovableMember>(
  selfMember: RemovableSelf | null,
  member: M,
  currentPubkey: string | undefined,
  isMyBot: (member: M) => boolean,
): boolean {
  return (
    (selfMember?.role === "admin" && member.pubkey !== currentPubkey) ||
    (selfMember?.role === "owner" && member.pubkey !== currentPubkey) ||
    Boolean(selfMember && isMyBot(member)) ||
    member.pubkey === currentPubkey
  );
}
