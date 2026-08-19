import { nip19 } from "nostr-tools";

const HEX_PUBKEY = /^[0-9a-fA-F]{64}$/;
const INVALID_PUBLIC_KEY = "Invalid public key";
const SECP256K1_FIELD = BigInt(
  "0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f",
);

function modPow(base: bigint, exponent: bigint, modulus: bigint): bigint {
  let result = 1n;
  let factor = base % modulus;
  let remaining = exponent;
  while (remaining > 0n) {
    if (remaining & 1n) result = (result * factor) % modulus;
    factor = (factor * factor) % modulus;
    remaining >>= 1n;
  }
  return result;
}

/** BIP-340 public keys are x coordinates that must lift to secp256k1. */
function isValidXOnlyPublicKey(hex: string): boolean {
  if (!HEX_PUBKEY.test(hex)) return false;
  const x = BigInt(`0x${hex}`);
  if (x >= SECP256K1_FIELD) return false;
  const ySquared = (x * x * x + 7n) % SECP256K1_FIELD;
  const y = modPow(ySquared, (SECP256K1_FIELD + 1n) >> 2n, SECP256K1_FIELD);
  return (y * y) % SECP256K1_FIELD === ySquared;
}

/** Convert a protocol pubkey to the canonical human-facing npub form. */
export function formatNpub(pubkey: string): string {
  const value = pubkey.trim();

  if (HEX_PUBKEY.test(value)) {
    const normalized = value.toLowerCase();
    return isValidXOnlyPublicKey(normalized)
      ? nip19.npubEncode(normalized)
      : INVALID_PUBLIC_KEY;
  }

  try {
    const decoded = nip19.decode(value);
    if (
      decoded.type === "npub" &&
      typeof decoded.data === "string" &&
      isValidXOnlyPublicKey(decoded.data)
    ) {
      return nip19.npubEncode(decoded.data.toLowerCase());
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
