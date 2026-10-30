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
