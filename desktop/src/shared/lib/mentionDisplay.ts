import { truncatePubkey } from "./pubkey";

/** Compact only a bound mention's key; its literal label remains authoritative. */
export function formatMentionDisplayLabel(
  label: string,
  pubkey: string | undefined,
): string {
  if (!pubkey || !/^[0-9a-f]{64}$/i.test(pubkey)) return label;
  if (label.toLowerCase() === pubkey.toLowerCase()) {
    return truncatePubkey(label);
  }
  const qualified = label.match(
    /^(.*) \(([0-9a-f]{64})\)((?: (?:[2-9]|[1-9][0-9]+))?)$/i,
  );
  if (qualified?.[2].toLowerCase() !== pubkey.toLowerCase()) return label;
  return `${qualified[1]} (${truncatePubkey(qualified[2])})${qualified[3]}`;
}
