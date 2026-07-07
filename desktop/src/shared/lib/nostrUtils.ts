import { decode, npubEncode } from "nostr-tools/nip19";
import { getPublicKey } from "nostr-tools/pure";

/**
 * Convert a hex-encoded Nostr public key to its npub (bech32) representation.
 *
 * @param hexPubkey — 64-character hex string
 * @returns npub1… bech32-encoded public key
 */
export function pubkeyToNpub(hexPubkey: string): string {
  return npubEncode(hexPubkey);
}

/**
 * Like `pubkeyToNpub`, but returns null instead of throwing on malformed
 * input. For display surfaces that must degrade gracefully.
 */
export function safeNpub(pubkey: string): string | null {
  try {
    return npubEncode(pubkey);
  } catch {
    return null;
  }
}

/**
 * Decode a bech32 nsec string and derive the matching npub. Returns null if
 * the input is not a syntactically valid `nsec1…` (does NOT throw — this is
 * intended for live form validation where the user is mid-typing).
 *
 * The input is trimmed first; surrounding whitespace from copy-paste or a
 * dropped `.key` file is tolerated.
 */
export function nsecToNpub(nsec: string): string | null {
  const trimmed = nsec.trim();
  if (!trimmed.startsWith("nsec1")) {
    return null;
  }
  try {
    const decoded = decode(trimmed);
    if (decoded.type !== "nsec") {
      return null;
    }
    const pubkeyHex = getPublicKey(decoded.data);
    return npubEncode(pubkeyHex);
  } catch {
    return null;
  }
}
