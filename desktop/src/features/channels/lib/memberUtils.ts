import type { ChannelMember } from "@/shared/api/types";
import {
  canonicalNpub,
  normalizePubkey,
  truncateNpub,
  UNAVAILABLE_KEY_LABEL,
} from "@/shared/lib/pubkey";

export const roleOrder: Record<ChannelMember["role"], number> = {
  owner: 0,
  admin: 1,
  member: 2,
  guest: 3,
  bot: 4,
};

export function formatMemberName(
  member: ChannelMember,
  currentPubkey?: string,
) {
  if (currentPubkey && member.pubkey === currentPubkey) {
    return "You";
  }

  return member.displayName ?? truncateNpub(member.pubkey);
}

/**
 * Ordering surface for a member's name: the authored name when present,
 * else the FULL canonical npub. Separate from `formatMemberName` — the
 * compact `npub1abcd…wxyz` label is display-only, and ordering on it would
 * collapse two distinct identities that merely share a prefix and tail.
 * Undisplayable keys keep the neutral label as their surface, exactly as
 * they render.
 */
function memberNameSurface(member: ChannelMember): string {
  return (
    member.displayName ?? canonicalNpub(member.pubkey) ?? UNAVAILABLE_KEY_LABEL
  );
}

/**
 * Full identity key used to break name-surface ties: the canonical npub of
 * a valid identity, else the normalized raw key so every tie is decided.
 * An ordering key only — never rendered or copied.
 */
function memberIdentityKey(member: ChannelMember): string {
  return canonicalNpub(member.pubkey) ?? normalizePubkey(member.pubkey);
}

/**
 * Shared name/key ordering authority for the roster comparators.
 *
 * Authored names keep the existing `localeCompare` semantics; unnamed
 * members order by their full canonical npub (the compact label stays
 * display-only); collation-equal surfaces — duplicate authored names,
 * matching labels, invalid keys — break by full identity key, so the
 * incoming membership-event order is never the tie policy. Comparators
 * layer their own role/current-user precedence around this stage; this
 * helper owns only the name ordering.
 */
export function compareMemberNames(
  left: ChannelMember,
  right: ChannelMember,
): number {
  const surfaceDelta = memberNameSurface(left).localeCompare(
    memberNameSurface(right),
  );
  if (surfaceDelta !== 0) {
    return surfaceDelta;
  }

  const leftKey = memberIdentityKey(left);
  const rightKey = memberIdentityKey(right);
  if (leftKey === rightKey) {
    return 0;
  }
  return leftKey < rightKey ? -1 : 1;
}

export function compareMembersByRole(
  left: ChannelMember,
  right: ChannelMember,
  currentPubkey?: string,
): number {
  if (currentPubkey && left.pubkey === currentPubkey) {
    return -1;
  }
  if (currentPubkey && right.pubkey === currentPubkey) {
    return 1;
  }
  const roleDelta = roleOrder[left.role] - roleOrder[right.role];
  if (roleDelta !== 0) {
    return roleDelta;
  }
  return compareMemberNames(left, right);
}

/**
 * Who may remove `member` from a channel.
 *
 * The two agent-ownership inputs are deliberately separate signals:
 *
 * - `isLocallyManagedBot` is **key custody** — this desktop holds the agent's
 *   seckey and has it in the managed-agent registry.
 * - `viewerIsDeclaredOwner` is **NIP-OA declared ownership** — the relay says
 *   the viewer owns this agent. `UserProfilePanel` already keys off exactly
 *   this (via `ownsAuthorAgent`) to decide whether to paint owner-scoped
 *   actions.
 *
 * Gating removal on custody alone stranded memberships: an agent the relay
 * declares the viewer owns, but which has fallen out of the local registry,
 * showed as theirs in the profile panel while the member list offered no way
 * to remove it. Membership removal is addressed by pubkey and enforced
 * server-side, so declared ownership is the right authority here — the local
 * registry is a cache, not the boundary.
 */
export function canRemoveChannelMember(input: {
  memberPubkey: string;
  memberRole: ChannelMember["role"];
  selfRole: ChannelMember["role"] | undefined;
  currentPubkey: string | undefined;
  isLocallyManagedBot: boolean;
  viewerIsDeclaredOwner: boolean;
}): boolean {
  const {
    memberPubkey,
    memberRole,
    selfRole,
    currentPubkey,
    isLocallyManagedBot,
    viewerIsDeclaredOwner,
  } = input;

  // Leaving is always yours to do, member or not.
  if (memberPubkey === currentPubkey) return true;
  // Every remaining path requires the viewer to be in the channel.
  if (!selfRole) return false;

  if (selfRole === "admin") return true;
  if (selfRole === "owner" && memberRole !== "owner") return true;
  return isLocallyManagedBot || viewerIsDeclaredOwner;
}
