import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

function isBestiePairDm(
  channel: Channel,
  currentPubkey: string,
  bestiePubkey: string,
) {
  const expectedParticipants = new Set([
    normalizePubkey(currentPubkey),
    normalizePubkey(bestiePubkey),
  ]);
  const participants = new Set(channel.participantPubkeys.map(normalizePubkey));
  return (
    participants.size === expectedParticipants.size &&
    [...expectedParticipants].every((pubkey) => participants.has(pubkey))
  );
}

export function filterBestieDmChannels(
  channels: Channel[],
  currentPubkey: string | undefined,
  bestiePubkey: string | null,
) {
  if (!currentPubkey || !bestiePubkey) return channels;

  return channels.filter(
    (channel) => !isBestiePairDm(channel, currentPubkey, bestiePubkey),
  );
}

/** Find the one-to-one Bestie DM in a channels cache (for instant popover hydrate). */
export function findBestieDmChannel(
  channels: readonly Channel[],
  currentPubkey: string | undefined,
  bestiePubkey: string | null | undefined,
): Channel | null {
  if (!currentPubkey || !bestiePubkey) return null;
  return (
    channels.find((channel) =>
      isBestiePairDm(channel, currentPubkey, bestiePubkey),
    ) ?? null
  );
}
