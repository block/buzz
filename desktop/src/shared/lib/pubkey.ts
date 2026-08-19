import { parsePubkeyInput, safeNpub } from "@/shared/lib/nostrUtils";

/**
 * Canonical pubkey normalisation for comparisons and protocol boundaries.
 *
 * Valid npub and legacy-hex inputs converge on lowercase protocol hex. The
 * lowercase fallback preserves the historical behavior for partial or
 * synthetic values used by local state and tests.
 */
export function normalizePubkey(pubkey: string): string {
  return parsePubkeyInput(pubkey) ?? pubkey.trim().toLowerCase();
}

/**
 * The ONE canonical compact display form for a pubkey: `npub1abc…wxyz`.
 *
 * A truncated pubkey is a recognition aid, never an identity proof — vanity
 * grinders forge short prefixes cheaply. Surfaces where the user makes a
 * trust decision must show the full npub (see `<PubKey variant="full">`).
 * Do not hand-roll `pubkey.slice(…)` display forms; `check-pubkey-truncation`
 * fails the build if one sneaks in outside this module.
 */
export function truncatePubkey(pubkey: string): string {
  const npub = safeNpub(pubkey);
  if (!npub) return "Invalid public key";
  return truncateIdentifier(npub);
}

/**
 * Compact a protocol-level hexadecimal identifier such as an event id or
 * Blossom blob hash. Public keys must use `truncatePubkey` instead.
 */
export function truncateHexId(value: string): string {
  return truncateIdentifier(value.trim().toLowerCase());
}

function truncateIdentifier(value: string): string {
  if (value.length <= 12) return value;
  return `${value.slice(0, 8)}…${value.slice(-4)}`;
}
