import { hexToBytes } from "@noble/hashes/utils.js";
import { decode, npubEncode, nsecEncode } from "nostr-tools/nip19";
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

/** 32-byte key material as 64 hex chars (pubkey or secret; case-insensitive). */
export const HEX_64_REGEX = /^[0-9a-fA-F]{64}$/;

/**
 * Parse user-entered public key input — either a 64-character hex pubkey or
 * a bech32 `npub1…` string — into a lowercase hex pubkey. Returns null for
 * anything else (does NOT throw — intended for live form validation).
 *
 * The input is trimmed first; surrounding whitespace from copy-paste is
 * tolerated.
 */
export function parsePubkeyInput(input: string): string | null {
  const trimmed = input.trim().toLowerCase();
  if (HEX_64_REGEX.test(trimmed)) {
    return trimmed;
  }
  if (trimmed.startsWith("npub1")) {
    try {
      const decoded = decode(trimmed);
      if (decoded.type === "npub") {
        return decoded.data;
      }
    } catch {
      return null;
    }
  }
  return null;
}

/**
 * Normalize a pasted private key to bech32 `nsec1…`.
 *
 * Accepts either `nsec1…` or a 64-char hex secret (what `buzz-admin
 * generate-key` prints / `BUZZ_PRIVATE_KEY` accepts). Returns null for
 * anything else — does not throw; intended for live form validation.
 */
export function normalizePrivateKeyToNsec(input: string): string | null {
  const trimmed = input.trim();
  if (trimmed.startsWith("nsec1")) {
    try {
      const decoded = decode(trimmed);
      if (decoded.type !== "nsec") {
        return null;
      }
      return trimmed;
    } catch {
      return null;
    }
  }
  if (HEX_64_REGEX.test(trimmed)) {
    try {
      return nsecEncode(hexToBytes(trimmed.toLowerCase()));
    } catch {
      return null;
    }
  }
  return null;
}

/**
 * Decode a private key (bech32 `nsec1…` or 64-char hex) and derive the
 * matching npub. Returns null if the input is not a syntactically valid
 * secret (does NOT throw — this is intended for live form validation where
 * the user is mid-typing).
 *
 * The input is trimmed first; surrounding whitespace from copy-paste or a
 * dropped `.key` file is tolerated.
 */
export function nsecToNpub(nsec: string): string | null {
  const normalized = normalizePrivateKeyToNsec(nsec);
  if (!normalized) {
    return null;
  }
  try {
    const decoded = decode(normalized);
    if (decoded.type !== "nsec") {
      return null;
    }
    const pubkeyHex = getPublicKey(decoded.data);
    return npubEncode(pubkeyHex);
  } catch {
    return null;
  }
}
