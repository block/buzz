import { decode, npubEncode } from "nostr-tools/nip19";
import { getPublicKey } from "nostr-tools/pure";

const HEX_PUBKEY_REGEX = /^[0-9a-fA-F]{64}$/;
const NOSTR_URI_PREFIX = "nostr:";
const NSEC_TOKEN_PATTERN = /(?:^|[^a-z0-9])(?:nostr:)?nsec1/i;
const SECP256K1_FIELD = BigInt(
  "0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f",
);

function hasMixedAsciiCase(value: string): boolean {
  return /[a-z]/.test(value) && /[A-Z]/.test(value);
}

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
  const x = BigInt(`0x${hex}`);
  if (x >= SECP256K1_FIELD) return false;
  const ySquared = (x * x * x + 7n) % SECP256K1_FIELD;
  const y = modPow(ySquared, (SECP256K1_FIELD + 1n) >> 2n, SECP256K1_FIELD);
  return (y * y) % SECP256K1_FIELD === ySquared;
}

/**
 * Parse user-entered public key input into the lowercase hex representation
 * required by Nostr event builders and filters. Canonical `npub1…` and
 * `nostr:npub1…` inputs are preferred; legacy 64-character hex remains
 * readable for compatibility at import/paste/deep-link boundaries.
 *
 * The input is trimmed first; surrounding whitespace from copy-paste is
 * tolerated.
 */
export function parsePubkeyInput(input: string): string | null {
  let trimmed = input.trim();
  if (
    trimmed.slice(0, NOSTR_URI_PREFIX.length).toLowerCase() === NOSTR_URI_PREFIX
  ) {
    trimmed = trimmed.slice(NOSTR_URI_PREFIX.length);
  }
  if (HEX_PUBKEY_REGEX.test(trimmed)) {
    const normalized = trimmed.toLowerCase();
    return isValidXOnlyPublicKey(normalized) ? normalized : null;
  }
  if (trimmed.slice(0, 5).toLowerCase() === "npub1") {
    if (hasMixedAsciiCase(trimmed)) return null;
    try {
      const decoded = decode(trimmed.toLowerCase());
      if (decoded.type === "npub" && isValidXOnlyPublicKey(decoded.data)) {
        return decoded.data.toLowerCase();
      }
    } catch {
      return null;
    }
  }
  return null;
}

/**
 * Convert an npub, `nostr:npub`, or legacy hex public key into the canonical
 * lowercase `npub1…` representation used by Buzz UI and portable app data.
 */
export function pubkeyToNpub(pubkey: string): string {
  const hexPubkey = parsePubkeyInput(pubkey);
  if (!hexPubkey) {
    throw new Error("Invalid Nostr public key");
  }
  return npubEncode(hexPubkey);
}

/**
 * Like `pubkeyToNpub`, but returns null instead of throwing on malformed
 * input. This helper is idempotent for canonical npubs.
 */
export function safeNpub(pubkey: string): string | null {
  try {
    return pubkeyToNpub(pubkey);
  } catch {
    return null;
  }
}

/**
 * Detect nsec-shaped text before it can cross a search or logging boundary.
 * This deliberately checks the prefix rather than validating the checksum:
 * malformed, truncated, uppercase, and NIP-21-wrapped secrets must all fail
 * closed instead of being relayed as ordinary search text.
 */
export function containsNsecShapedInput(input: string): boolean {
  return NSEC_TOKEN_PATTERN.test(input.trim());
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
  if (trimmed.slice(0, 5).toLowerCase() !== "nsec1") {
    return null;
  }
  if (hasMixedAsciiCase(trimmed)) return null;
  try {
    const decoded = decode(trimmed.toLowerCase());
    if (decoded.type !== "nsec") {
      return null;
    }
    const pubkeyHex = getPublicKey(decoded.data);
    return npubEncode(pubkeyHex);
  } catch {
    return null;
  }
}
