/**
 * Normalizes a relay URL for use in storage keys.
 * Trim, strip trailing slashes, lowercase — ensures equivalent URLs map to
 * the same key regardless of formatting differences.
 */
export function normalizeRelayUrl(relayUrl: string): string {
  return relayUrl.trim().replace(/\/+$/, "").toLowerCase();
}
