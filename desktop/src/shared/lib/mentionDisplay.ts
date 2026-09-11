import { truncateNpub, truncatePubkey } from "./pubkey";

type KeyCompaction = (key: string) => string;

function compactMentionDisplayLabel(
  label: string,
  pubkey: string | undefined,
  compactKey: KeyCompaction,
): string {
  if (!pubkey || !/^[0-9a-f]{64}$/i.test(pubkey)) return label;
  if (label.toLowerCase() === pubkey.toLowerCase()) {
    return compactKey(label);
  }
  const qualified = label.match(
    /^(.*) \(([0-9a-f]{64})\)((?: (?:[2-9]|[1-9][0-9]+))?)$/i,
  );
  if (qualified?.[2].toLowerCase() !== pubkey.toLowerCase()) return label;
  return `${qualified[1]} (${compactKey(qualified[2])})${qualified[3]}`;
}

/** Compact only a bound mention's key; its literal label remains authoritative. */
export function formatMentionDisplayLabel(
  label: string,
  pubkey: string | undefined,
): string {
  return compactMentionDisplayLabel(label, pubkey, truncateNpub);
}

/**
 * The pre-npub key compaction a chip rendered before keys displayed as npub.
 * Retired from rendering; kept byte-exact so clipboard validation can still
 * recognize whole chips copied by an older Buzz, re-binding them to the exact
 * identity their record declares instead of degrading them to plain text.
 */
export function formatLegacyMentionDisplayLabel(
  label: string,
  pubkey: string | undefined,
): string {
  return compactMentionDisplayLabel(label, pubkey, truncatePubkey);
}
