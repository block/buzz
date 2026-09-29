import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function filterBestieDmChannels(
  channels: Channel[],
  currentPubkey: string | undefined,
  bestiePubkey: string | null,
) {
  if (!currentPubkey || !bestiePubkey) return channels;

  const expectedParticipants = new Set([
    normalizePubkey(currentPubkey),
    normalizePubkey(bestiePubkey),
  ]);

  return channels.filter((channel) => {
    const participants = new Set(
      channel.participantPubkeys.map(normalizePubkey),
    );
    const isBestiePair =
      participants.size === expectedParticipants.size &&
      [...expectedParticipants].every((pubkey) => participants.has(pubkey));
    return !isBestiePair;
  });
}
