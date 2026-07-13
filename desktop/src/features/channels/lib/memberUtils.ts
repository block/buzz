import type { ChannelMember } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";

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

  return member.displayName ?? truncatePubkey(member.pubkey);
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
  return formatMemberName(left).localeCompare(formatMemberName(right));
}
