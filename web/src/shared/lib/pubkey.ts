import { nip19 } from "nostr-tools";

const HEX_PUBKEY = /^[0-9a-fA-F]{64}$/;
const INVALID_PUBLIC_KEY = "Invalid public key";

/** Convert a protocol pubkey to the canonical human-facing npub form. */
export function formatNpub(pubkey: string): string {
  const value = pubkey.trim();

  if (HEX_PUBKEY.test(value)) {
    return nip19.npubEncode(value.toLowerCase());
  }

  try {
    const decoded = nip19.decode(value);
    if (decoded.type === "npub" && typeof decoded.data === "string") {
      return nip19.npubEncode(decoded.data);
    }
  } catch {
    // Fall through to a stable sentinel; never echo a malformed raw identity.
  }

  return INVALID_PUBLIC_KEY;
}

/**
 * The ONE canonical compact display form for a pubkey: `npub1abc…wxyz`.
 * Mirrors desktop's `@/shared/lib/pubkey`. A truncated pubkey is a
 * recognition aid, never an identity proof — security decisions need the
 * full npub.
 */
export function truncatePubkey(pubkey: string): string {
  const npub = formatNpub(pubkey);
  if (npub.length <= 12) {
    return npub;
  }
  return `${npub.slice(0, 8)}…${npub.slice(-4)}`;
}

/** Two payload characters for an avatar glyph, derived from canonical npub. */
export function pubkeyAvatarLabel(pubkey: string): string {
  const npub = formatNpub(pubkey);
  return npub.startsWith("npub1") ? npub.slice(5, 7) : npub.slice(0, 2);
}
