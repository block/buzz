import type { Channel, ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type CanStartHuddleInput = {
  channel: Channel;
  currentPubkey?: string;
  selfMember: ChannelMember | null;
};

export function canStartHuddleInChannel({
  channel,
  currentPubkey,
  selfMember,
}: CanStartHuddleInput): boolean {
  if (channel.archivedAt !== null) {
    return false;
  }

  if (channel.channelType === "dm") {
    if (channel.isMember) {
      return true;
    }

    if (!currentPubkey) {
      return false;
    }

    const normalizedCurrentPubkey = normalizePubkey(currentPubkey);
    return channel.participantPubkeys.some(
      (pubkey) => normalizePubkey(pubkey) === normalizedCurrentPubkey,
    );
  }

  return channel.visibility === "open" || selfMember !== null;
}
