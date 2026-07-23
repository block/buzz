import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Return the semantic recipients for an outgoing message.
 *
 * Stream messages notify only explicit mentions. A DM addresses every other
 * participant, so it must carry recipient `p` tags even when the composer text
 * contains no `@mention`. Agent harnesses and human notification subscriptions
 * both rely on those tags.
 */
export function messageMentionPubkeys(
  channel: Channel,
  senderPubkey: string,
  explicitMentions: readonly string[] = [],
): string[] {
  const candidates =
    channel.channelType === "dm"
      ? [
          ...explicitMentions,
          ...channel.memberPubkeys,
          ...channel.participantPubkeys,
        ]
      : explicitMentions;
  const sender = normalizePubkey(senderPubkey);

  return [...new Set(candidates.map(normalizePubkey))].filter(
    (pubkey) => pubkey.length > 0 && pubkey !== sender,
  );
}
