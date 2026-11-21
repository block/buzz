/**
 * Normalizes a relay URL for use in storage keys.
 * Trim, strip trailing slashes, lowercase — ensures equivalent URLs map to
 * the same key regardless of formatting differences.
 */
export function normalizeRelayUrl(relayUrl: string): string {
  return relayUrl.trim().replace(/\/+$/, "").toLowerCase();
}

/** Canonicalize relay endpoint identity while preserving path and query case. */
export function canonicalizeRelayUrl(relayUrl: string): string {
  const trimmed = relayUrl.trim();
  try {
    const url = new URL(trimmed);
    const suffix = `${url.search}${url.hash}`;
    const endpoint = suffix ? url.href.slice(0, -suffix.length) : url.href;
    return `${endpoint.replace(/\/+$/, "")}${suffix}`;
  } catch {
    return trimmed.replace(/\/+$/, "");
  }
}
