/** A name the composer can map back to a pubkey when `@Name` is sent. */
export type MentionRecord = {
  name: string;
  pubkey: string;
  isAgent: boolean;
};

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Records whose `@Name` still appears in the text at send time — accepting
 * a suggestion only counts if the mention survived editing. Longest names
 * match first and consume their span, so `@amp` never claims a mention of
 * `@amp (local)`. Boundaries mirror `withAgentMention`: the `@` must open a
 * word (so `email@example.com` is not a mention).
 */
export function extractMentions(
  text: string,
  records: readonly MentionRecord[],
): MentionRecord[] {
  const found: MentionRecord[] = [];
  const seenPubkeys = new Set<string>();
  const ordered = [...records]
    .filter((record) => record.name)
    .sort((left, right) => right.name.length - left.name.length);
  let remaining = text;
  for (const record of ordered) {
    const pattern = new RegExp(
      `(^|[\\s([{])@${escapeRegExp(record.name)}(?=$|[\\s,.;:!?)\\]}])`,
      "gi",
    );
    if (!pattern.test(remaining)) continue;
    remaining = remaining.replace(pattern, "$1");
    const pubkey = record.pubkey.toLowerCase();
    if (seenPubkeys.has(pubkey)) continue;
    seenPubkeys.add(pubkey);
    found.push(record);
  }
  return found;
}
