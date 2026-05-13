/**
 * Pure helpers for the inbound author gate UI.
 *
 * The Rust side is the canonical validator (see
 * `desktop/src-tauri/src/managed_agents/types.rs::validate_respond_to_allowlist`).
 * These helpers exist to give the UI immediate, inline feedback before the
 * round-trip, and to normalize input so the Rust validator sees clean data.
 */

const HEX_64 = /^[0-9a-f]{64}$/i;

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
 * - Trims and lowercases each entry.
 * - Validates each entry is exactly 64 hex chars.
 * - Deduplicates while preserving insertion order.
 */
export function parsePubkeyInput(raw: string): ParsedAllowlist {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const piece of raw.split(/[\s,]+/)) {
    const trimmed = piece.trim();
    if (trimmed.length === 0) continue;
    if (!HEX_64.test(trimmed)) {
      invalid.push(trimmed);
      continue;
    }
    const lower = trimmed.toLowerCase();
    if (!seen.has(lower)) {
      seen.add(lower);
      valid.push(lower);
    }
  }
  return { valid, invalid };
}

/**
 * Merge an existing allowlist with newly-added pubkeys, normalizing and
 * deduplicating without reordering existing entries.
 */
export function mergeAllowlist(existing: string[], add: string[]): string[] {
  const seen = new Set(existing.map((p) => p.toLowerCase()));
  const out = [...existing.map((p) => p.toLowerCase())];
  for (const candidate of add) {
    const lower = candidate.toLowerCase();
    if (!HEX_64.test(lower) || seen.has(lower)) continue;
    seen.add(lower);
    out.push(lower);
  }
  return out;
}
