import type { TemplateMemberEntry } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const HEX64 = /^[0-9a-f]{64}$/;

/**
 * Canonical pubkey list for a template's direct members: normalized
 * (lowercase, trimmed), deduplicated case-insensitively, first-seen order,
 * with malformed entries dropped. Pure so the selection logic is testable
 * without React context or the Tauri command surface.
 */
export function templateMemberPubkeys(
  members: readonly TemplateMemberEntry[] | undefined,
): string[] {
  const seen = new Set<string>();
  const pubkeys: string[] = [];
  for (const entry of members ?? []) {
    const pk = normalizePubkey(entry.pubkey);
    if (!HEX64.test(pk) || seen.has(pk)) continue;
    seen.add(pk);
    pubkeys.push(pk);
  }
  return pubkeys;
}
