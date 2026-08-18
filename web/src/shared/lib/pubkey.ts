/**
 * The ONE canonical compact display form for a pubkey: `abcd1234…wxyz`.
 * Mirrors desktop's `@/shared/lib/pubkey`. A truncated pubkey is a
 * recognition aid, never an identity proof — security decisions need the
 * full npub.
 */
export function truncatePubkey(pubkey: string): string {
  if (pubkey.length <= 12) {
    return pubkey;
  }
  return `${pubkey.slice(0, 8)}…${pubkey.slice(-4)}`;
}
