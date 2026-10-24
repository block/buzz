import {
  canonicalMentionLabel,
  type MentionIdentity,
} from "./mentionClipboard";

/**
 * Whether trusted Buzz state vouches for a copied `label → pubkey` pair.
 *
 * Clipboard HTML is attacker-authored: any page can carry
 * `<span data-mention-pubkey="<their key>" data-mention-label="John Smith">
 * @John Smith</span>`, and that pastes as a plausible, *visible* mention.
 * Visibility only proves the user saw a name — not that the name belongs to
 * the key beside it. A marker Buzz writes proves less still, since an attacker
 * can write the same marker.
 *
 * So the pair itself has to be checked against state the community supplied:
 * the pubkey's own profile aliases, or a directory entry naming it. A pair
 * nothing vouches for binds nothing — the words paste as readable text and no
 * `p` tag carries the key. That is the fail-closed direction: the cost is a
 * mention that stays plain, never an outbound tag naming the wrong person.
 */

/**
 * Does `label` name one of `aliases`?
 *
 * Compared through `canonicalMentionLabel`, so a legitimate copy is not
 * refused over the casing, padding, or U+00A0 a pasteboard round trip leaves
 * on the declared label. Generated same-name qualifiers additionally require
 * the embedded full key to match the record AND the base alias to be trusted.
 * The qualifier alone is never identity evidence.
 */
export function isTrustedMentionLabel(
  label: string,
  aliases: Iterable<string>,
  pubkey?: string,
): boolean {
  const wanted = canonicalMentionLabel(label);
  if (!wanted) return false;
  const qualified = wanted.match(
    /^(.*) \(([0-9a-f]{64})\)(?: (?:[2-9]|[1-9][0-9]+))?$/,
  );
  const baseAlias =
    qualified?.[2] === pubkey?.toLowerCase() ? qualified?.[1] : undefined;
  for (const alias of aliases) {
    const trusted = canonicalMentionLabel(alias);
    if (trusted === wanted || (baseAlias && trusted === baseAlias)) return true;
  }
  return false;
}

/**
 * Split records by whether locally held trusted state already vouches for
 * them, so only the remainder costs a relay round trip.
 *
 * `resolveLocalAliases` answers for a normalized pubkey with every name local
 * trusted state knows it by. An empty answer is "not known here", never
 * "refuted" — the caller escalates those to the relay, which is the only
 * source that can speak for a pubkey no local directory has seen.
 */
export function partitionMentionIdentitiesByLocalTrust(
  records: readonly MentionIdentity[],
  resolveLocalAliases: (pubkey: string) => readonly string[],
): { trusted: MentionIdentity[]; unresolved: MentionIdentity[] } {
  const trusted: MentionIdentity[] = [];
  const unresolved: MentionIdentity[] = [];
  for (const record of records) {
    if (
      isTrustedMentionLabel(
        record.label,
        resolveLocalAliases(record.pubkey),
        record.pubkey,
      )
    ) {
      trusted.push(record);
    } else {
      unresolved.push(record);
    }
  }
  return { trusted, unresolved };
}
