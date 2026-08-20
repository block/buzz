import { nip19 } from "nostr-tools";

import { maskMarkdownCode } from "./hasMention";

/**
 * NIP-21 `nostr:` URI naming a person: `npub` (bare pubkey) or `nprofile`
 * (pubkey plus relay hints). Event references (`note`, `nevent`, `naddr`) name
 * content rather than a recipient and are deliberately not matched.
 *
 * The data part uses the bech32 charset, which excludes `1`, `b`, `i`, and
 * `o` — matching it exactly stops the match at surrounding punctuation.
 */
const NOSTR_PROFILE_URI_SOURCE =
  "nostr:((?:npub|nprofile)1[qpzry9x8gf2tvdw0s3jn54khce6mua7l]+)";

function pubkeyFromEntity(entity: string): string | null {
  let decoded: ReturnType<typeof nip19.decode>;
  try {
    decoded = nip19.decode(entity);
  } catch {
    // Truncated or corrupted bech32 — not a usable recipient.
    return null;
  }

  const pubkey =
    decoded.type === "npub"
      ? decoded.data
      : decoded.type === "nprofile"
        ? decoded.data.pubkey
        : null;

  if (typeof pubkey !== "string") return null;
  const normalized = pubkey.trim().toLowerCase();
  return /^[0-9a-f]{64}$/.test(normalized) ? normalized : null;
}

/**
 * Extract recipient pubkeys from NIP-21 `nostr:` URIs in composer text.
 *
 * A pasted `nostr:npub1…` addresses someone just as `@Name` does, so it must
 * become a recipient `p` tag — without one the mention is decorative and no
 * notification or agent wake-up happens.
 *
 * Code is masked first, so a URI inside backticks or a fenced block is left
 * alone. That mirrors `getMentionOffsets`, which already refuses to read
 * `@Name` out of code.
 *
 * Returns lowercase 64-char hex pubkeys in first-appearance order, deduped.
 */
export function extractNostrUriPubkeys(text: string): string[] {
  if (!text.includes("nostr:")) return [];

  const pattern = new RegExp(NOSTR_PROFILE_URI_SOURCE, "g");
  const masked = maskMarkdownCode(text);
  const pubkeys: string[] = [];
  const seen = new Set<string>();

  let match = pattern.exec(masked);
  while (match !== null) {
    const pubkey = pubkeyFromEntity(match[1]);
    if (pubkey && !seen.has(pubkey)) {
      seen.add(pubkey);
      pubkeys.push(pubkey);
    }
    match = pattern.exec(masked);
  }

  return pubkeys;
}
