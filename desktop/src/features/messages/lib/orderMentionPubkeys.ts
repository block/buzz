import { getMentionOffset } from "@/features/messages/lib/hasMention";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function orderMentionPubkeysByText(
  text: string,
  mentionPubkeysByName: Readonly<Record<string, string>> | undefined,
  isEligible: (pubkey: string) => boolean,
): string[] {
  if (!mentionPubkeysByName) return [];

  const earliestOffsetByPubkey = new Map<string, number>();
  for (const [name, pubkey] of Object.entries(mentionPubkeysByName)) {
    const normalized = normalizePubkey(pubkey);
    const offset = getMentionOffset(text, name);
    if (offset === null || !isEligible(normalized)) continue;

    const previousOffset = earliestOffsetByPubkey.get(normalized);
    if (previousOffset === undefined || offset < previousOffset) {
      earliestOffsetByPubkey.set(normalized, offset);
    }
  }

  return [...earliestOffsetByPubkey.entries()]
    .sort(([leftPubkey, leftOffset], [rightPubkey, rightOffset]) =>
      leftOffset === rightOffset
        ? leftPubkey.localeCompare(rightPubkey)
        : leftOffset - rightOffset,
    )
    .map(([pubkey]) => pubkey);
}
