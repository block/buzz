import { mentionOccurrences } from "@/shared/lib/mentionOccurrences";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function orderMentionPubkeysByText(
  text: string,
  mentionPubkeysByName: Readonly<Record<string, string>> | undefined,
  isEligible: (pubkey: string) => boolean,
  mentionNames: readonly string[] = Object.keys(mentionPubkeysByName ?? {}),
): string[] {
  if (!mentionPubkeysByName) return [];

  const earliestOffsetByPubkey = new Map<string, number>();
  const bindings = Object.entries(mentionPubkeysByName).map(
    ([displayName, pubkey]) => ({ displayName, pubkey }),
  );
  const boundNames = new Set(
    bindings.map((item) => item.displayName.toLowerCase()),
  );
  const candidates = [
    ...bindings,
    ...mentionNames
      .filter((name) => !boundNames.has(name.toLowerCase()))
      .map((displayName) => ({ displayName, pubkey: undefined })),
  ];
  for (const { start, candidates: winners } of mentionOccurrences(
    text,
    candidates,
  )) {
    if (
      new Set(
        winners.map((item) =>
          item.pubkey ? normalizePubkey(item.pubkey) : undefined,
        ),
      ).size !== 1
    )
      continue;
    if (!winners[0].pubkey) continue;
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
