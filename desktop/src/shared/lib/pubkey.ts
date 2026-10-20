import { decode, npubEncode } from "nostr-tools/nip19";

import { safeNpub } from "./nostrUtils";

/**
 * Canonical pubkey normalisation.
 *
 * Hex pubkeys are case-insensitive, but callers compare them with `===`.
 * Trimming guards against stray whitespace from user input or tag parsing.
 */
export function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

/** Neutral identity label for keys that cannot be encoded for display. */
export const UNAVAILABLE_KEY_LABEL = "Unavailable";

const HEX_64_REGEX = /^[0-9a-f]{64}$/;

/**
 * The ONE canonical compact display form for a hex string: `abcd1234…wxyz`.
 *
 * A truncated pubkey is a recognition aid, never an identity proof — vanity
 * grinders forge short prefixes cheaply. Surfaces where the user makes a
 * trust decision must show the full npub (see `<PubKey variant="full">`).
 * Do not hand-roll `pubkey.slice(…)` display forms; `check-pubkey-truncation`
 * fails the build if one sneaks in outside this module.
 *
 * Identity (pubkey) surfaces should use `truncateNpub` instead; this hex form
 * remains canonical for non-identity identifiers — event and blob IDs.
 */
export function truncatePubkey(pubkey: string): string {
  if (pubkey.length <= 12) {
    return pubkey;
  }
  return `${pubkey.slice(0, 8)}…${pubkey.slice(-4)}`;
}

/**
 * Canonical full npub for an identity key: a 64-char hex pubkey (any
 * case) or an already-npub string (checksum-validated) returns the
 * canonical npub; anything else returns null. Strict 64-char identity keys
 * only — `npubEncode` happily encodes short/degenerate payloads (even `""`),
 * which are not displayable identities.
 *
 * Bech32 casing is strict on the input as written: a lowercase `npub1…`
 * or an all-uppercase `NPUB1…` (both valid Bech32) returns the canonical
 * lowercase npub, while a mixed-case npub is invalid Bech32 and returns
 * null — `decode` enforces the all-lower/all-upper rule. This is
 * intentionally stricter than the parser (`parsePubkeyInput`), which
 * normalizes user input before decoding and so also accepts mixed-case
 * npubs; the two agree that the payload must be a 64-hex identity key and
 * that both valid casings above are acceptable input.
 */
export function canonicalNpub(pubkey: string): string | null {
  const trimmed = pubkey.trim();
  if (trimmed.startsWith("npub1") || trimmed.startsWith("NPUB1")) {
    try {
      const decoded = decode(trimmed);
      if (decoded.type !== "npub" || !HEX_64_REGEX.test(decoded.data)) {
        return null;
      }
      return npubEncode(decoded.data);
    } catch {
      return null;
    }
  }
  const normalized = normalizePubkey(trimmed);
  return HEX_64_REGEX.test(normalized) ? safeNpub(normalized) : null;
}

/**
 * The ONE canonical compact identity display for a pubkey: `npub1abcd…wxyz`
 * (first 8 + last 4 of the FULL npub).
 *
 * Identity surfaces render this form so a displayed prefix is always npub-
 * shaped; the underlying hex never leaks as the identity display. A
 * truncated key is a recognition aid, never an identity proof — trust
 * decisions use `<PubKey variant="full">` or the full npub directly. Invalid
 * keys render `UNAVAILABLE_KEY_LABEL`, never raw hex or raw input.
 */
export function truncateNpub(pubkey: string): string {
  const npub = canonicalNpub(pubkey);
  return npub === null ? UNAVAILABLE_KEY_LABEL : truncatePubkey(npub);
}
