/**
 * Pure helpers for the inbound author gate UI.
 *
 * The Rust side is the canonical validator (see
 * `desktop/src-tauri/src/managed_agents/types.rs::validate_respond_to_allowlist`).
 * These helpers exist to give the UI immediate, inline feedback before the
 * round-trip, and to normalize input so the Rust validator sees clean data.
 *
 * Entry pieces may be 64-char hex pubkeys or bech32 `npub1…` strings; both are
 * normalized to the canonical lowercase hex via the shared
 * `parsePubkeyInput`, so npub and hex spellings of the same key dedupe to one
 * entry (users copy npubs from profile/verify surfaces elsewhere in the app).
 */

import { parsePubkeyInput as parseCanonicalPubkey } from "@/shared/lib/nostrUtils";

export type ParsedAllowlist = {
  /** Successfully parsed entries — lowercase hex, deduplicated, in order. */
  valid: string[];
  /** Entries that failed validation, in their raw form. */
  invalid: string[];
};

/**
 * Parse a free-form pubkey-paste input (one per line, comma-separated, or
 * mixed whitespace) into a normalized allowlist. Matches the splitting
 * pattern used by `ChannelMemberInviteCard` so users have one mental model.
 *
 * - Splits on `/[\s,]+/`.
 * - Accepts 64-char hex (any case) or `npub1…` bech32 per piece, normalizing
 *   to the canonical lowercase hex pubkey.
 * - Deduplicates the canonical form while preserving insertion order.
 */
export function parsePubkeyInput(raw: string): ParsedAllowlist {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const piece of raw.split(/[\s,]+/)) {
    const trimmed = piece.trim();
    if (trimmed.length === 0) continue;
    const canonical = parseCanonicalPubkey(trimmed);
    if (canonical === null) {
      invalid.push(trimmed);
      continue;
    }
    if (!seen.has(canonical)) {
      seen.add(canonical);
      valid.push(canonical);
    }
  }
  return { valid, invalid };
}

/**
 * Merge an existing allowlist with newly-added pubkeys, normalizing and
 * deduplicating without reordering existing entries. Both hex and npub
 * spellings normalize to the canonical hex, so the same key cannot enter
 * twice regardless of the form it was added in.
 */
export function mergeAllowlist(existing: string[], add: string[]): string[] {
  const normalize = (pubkey: string): string =>
    parseCanonicalPubkey(pubkey) ?? pubkey.toLowerCase();
  const out = existing.map(normalize);
  const seen = new Set(out);
  for (const candidate of add) {
    const canonical = parseCanonicalPubkey(candidate);
    if (canonical === null || seen.has(canonical)) continue;
    seen.add(canonical);
    out.push(canonical);
  }
  return out;
}
