import { decode } from "nostr-tools/nip19";

/**
 * Canonical pubkey normalisation.
 *
 * Hex pubkeys are case-insensitive, but callers compare them with `===`.
 * Trimming guards against stray whitespace from user input or tag parsing.
 */
export function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

const HEX64 = /^[0-9a-f]{64}$/;

/**
 * Coerce a pubkey to canonical 64-char lowercase hex, accepting either hex
 * (any case, padded) or `npub1…`. Throws if the result is not a valid pubkey.
 *
 * Use at signing boundaries where a malformed value would otherwise be signed
 * into a tag and rejected downstream with an opaque error (e.g. the relay's
 * "mesh connect request missing #p target" when the tag value is unusable).
 * Failing here turns that into a clear, local error before the event is sent.
 */
export function canonicalPubkeyOrThrow(pubkey: string): string {
  const trimmed = pubkey.trim();
  if (trimmed.startsWith("npub1")) {
    const decoded = decode(trimmed);
    if (decoded.type !== "npub") {
      throw new Error(`invalid pubkey: ${trimmed}`);
    }
    return decoded.data;
  }
  const hex = trimmed.toLowerCase();
  if (!HEX64.test(hex)) {
    throw new Error(
      `invalid pubkey: expected 64-char hex or npub, got ${trimmed}`,
    );
  }
  return hex;
}
