/**
 * Pure helpers for the inbound author gate UI.
 *
 * The Rust side is the canonical validator (see
 * `desktop/src-tauri/src/managed_agents/types.rs::validate_respond_to_allowlist`).
 * These helpers exist to give the UI immediate, inline feedback before the
 * round-trip, and to normalize input so the Rust validator sees clean data.
 */

import { parsePubkeyInput as parseSinglePubkeyInput } from "@/shared/lib/nostrUtils";

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
 * - Accepts canonical npubs plus legacy hex at this paste boundary.
 * - Normalizes valid entries to lowercase hex for Nostr protocol internals.
 * - Deduplicates while preserving insertion order.
 */
export function parsePubkeyInput(raw: string): ParsedAllowlist {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const piece of raw.split(/[\s,]+/)) {
    const trimmed = piece.trim();
    if (trimmed.length === 0) continue;
    const pubkey = parseSinglePubkeyInput(trimmed);
    if (!pubkey) {
      invalid.push(trimmed);
      continue;
    }
    if (!seen.has(pubkey)) {
      seen.add(pubkey);
      valid.push(pubkey);
    }
  }
  return { valid, invalid };
}

/**
 * Merge an existing allowlist with newly-added pubkeys, normalizing and
 * deduplicating without reordering existing entries.
 */
export function mergeAllowlist(existing: string[], add: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const candidate of [...existing, ...add]) {
    const pubkey = parseSinglePubkeyInput(candidate);
    if (!pubkey || seen.has(pubkey)) continue;
    seen.add(pubkey);
    out.push(pubkey);
  }
  return out;
}
