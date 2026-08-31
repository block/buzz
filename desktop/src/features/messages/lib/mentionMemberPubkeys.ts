import type { Channel, ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { channelMemberPubkeySet } from "@/shared/lib/rosterDerivations";

/** Merge the dedicated roster with the active channel's signed projection. */
export function getMentionMemberPubkeys(
  channelId: string | null,
  channels: readonly Channel[] | undefined,
  members: ChannelMember[] | undefined,
): Set<string> {
  const pubkeys = new Set(
    members ? channelMemberPubkeySet(members) : undefined,
  );
  const activeChannel = channels?.find((channel) => channel.id === channelId);
  for (const pubkey of activeChannel?.memberPubkeys ?? []) {
    pubkeys.add(normalizePubkey(pubkey));
  }
  return pubkeys;
}
