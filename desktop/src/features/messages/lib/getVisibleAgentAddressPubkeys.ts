import { orderMentionPubkeysByText } from "@/features/messages/lib/orderMentionPubkeys";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Keep tag-backed address recipients visible without repeating ordinary inline
 * mentions already present in the message body.
 */
export function getVisibleAgentAddressPubkeys(
  body: string,
  addressedPubkeys: readonly string[],
  mentionPubkeysByName: Readonly<Record<string, string>> | undefined,
): string[] {
  const inlineMentionPubkeys = new Set(
    orderMentionPubkeysByText(body, mentionPubkeysByName, () => true),
  );

  return addressedPubkeys.filter(
    (pubkey) => !inlineMentionPubkeys.has(normalizePubkey(pubkey)),
  );
}
