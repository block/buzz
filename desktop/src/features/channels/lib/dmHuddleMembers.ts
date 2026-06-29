import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function getDmHuddleMemberPubkeys(
  channel: Channel | null,
  agentPubkeys: ReadonlySet<string> | undefined,
  currentPubkey: string | undefined,
) {
  if (channel?.channelType !== "dm" || !agentPubkeys) {
    return [];
  }

  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const seen = new Set<string>();

  return channel.participantPubkeys.filter((pubkey) => {
    const normalizedPubkey = normalizePubkey(pubkey);
    if (
      normalizedCurrentPubkey &&
      normalizedPubkey === normalizedCurrentPubkey
    ) {
      return false;
    }

    if (!agentPubkeys.has(normalizedPubkey) || seen.has(normalizedPubkey)) {
      return false;
    }

    seen.add(normalizedPubkey);
    return true;
  });
}

export function hasOtherDmParticipant(
  channel: Channel | null,
  currentPubkey: string | undefined,
) {
  if (channel?.channelType !== "dm") {
    return false;
  }

  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  return channel.participantPubkeys.some((pubkey) => {
    const normalizedPubkey = normalizePubkey(pubkey);
    return (
      !normalizedCurrentPubkey || normalizedPubkey !== normalizedCurrentPubkey
    );
  });
}
