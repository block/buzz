/**
 * Canonical pubkey normalisation.
 *
 * Hex pubkeys are case-insensitive, but callers compare them with `===`.
 * Trimming guards against stray whitespace from user input or tag parsing.
 */
export function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

/**
 * The ONE canonical compact display form for a pubkey: `abcd1234…wxyz`.
 *
 * A truncated pubkey is a recognition aid, never an identity proof — vanity
 * grinders forge short prefixes cheaply. Surfaces where the user makes a
 * trust decision must show the full npub (see `<PubKey variant="full">`).
 * Do not hand-roll `pubkey.slice(…)` display forms; `check-pubkey-truncation`
 * fails the build if one sneaks in outside this module.
 */
export function truncatePubkey(pubkey: string): string {
  if (pubkey.length <= 12) {
    return pubkey;
  }
  return `${pubkey.slice(0, 8)}…${pubkey.slice(-4)}`;
}
