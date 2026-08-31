import { mentionOccurrences } from "@/shared/lib/mentionOccurrences";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function orderMentionPubkeysByText(
  text: string,
  mentionPubkeysByName: Readonly<Record<string, string>> | undefined,
  isEligible: (pubkey: string) => boolean,
): string[] {
  if (!mentionPubkeysByName) return [];

  const earliestOffsetByPubkey = new Map<string, number>();
  const candidates = Object.entries(mentionPubkeysByName).map(
    ([displayName, pubkey]) => ({ displayName, pubkey }),
  );
  for (const { start, candidates: winners } of mentionOccurrences(
    text,
    candidates,
  )) {
    if (new Set(winners.map((item) => normalizePubkey(item.pubkey))).size !== 1)
      continue;
    const normalized = normalizePubkey(winners[0].pubkey);
    if (isEligible(normalized) && !earliestOffsetByPubkey.has(normalized))
      earliestOffsetByPubkey.set(normalized, start);
  }

  return [...earliestOffsetByPubkey.entries()]
    .sort(([leftPubkey, leftOffset], [rightPubkey, rightOffset]) =>
      leftOffset === rightOffset
        ? leftPubkey.localeCompare(rightPubkey)
        : leftOffset - rightOffset,
    )
    .map(([pubkey]) => pubkey);
}
