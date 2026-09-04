import { decode } from "nostr-tools/nip19";

import { maskMarkdownCode } from "@/features/messages/lib/hasMention";
import { normalizePubkey } from "@/shared/lib/pubkey";

const NIP27_NPUB_RE = /nostr:(npub1[02-9ac-hj-np-z]+)/gi;

function npubToHex(npub: string): string | null {
  try {
    const decoded = decode(npub);
    if (decoded.type !== "npub") {
      return null;
    }
    if (typeof decoded.data === "string") {
      return normalizePubkey(decoded.data);
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Extract pubkeys from NIP-27 `nostr:npub1…` URIs in content, skipping
 * Markdown code spans/blocks (same masking as `@mention` matching).
 */
export function extractNip27MentionPubkeys(text: string): string[] {
  if (!/nostr:npub1/i.test(text)) {
    return [];
  }

  const masked = maskMarkdownCode(text);
  const pubkeys: string[] = [];
  const seen = new Set<string>();
  NIP27_NPUB_RE.lastIndex = 0;
  for (const match of masked.matchAll(NIP27_NPUB_RE)) {
    const hex = npubToHex(match[1]);
    if (!hex || seen.has(hex)) {
      continue;
    }
    seen.add(hex);
    pubkeys.push(hex);
  }
  return pubkeys;
}

/**
 * For display, map known NIP-27 URIs back to `@displayName` so existing
 * remark mention highlighting keeps working. Unknown URIs are left intact.
 */
export function replaceNip27MentionsForDisplay(
  text: string,
  mentionPubkeysByName: Readonly<Record<string, string>> | undefined,
): string {
  if (!mentionPubkeysByName || !/nostr:npub1/i.test(text)) {
    return text;
  }

  const nameByPubkey = new Map<string, string>();
  for (const [name, pubkey] of Object.entries(mentionPubkeysByName)) {
    const normalized = normalizePubkey(pubkey);
    if (!nameByPubkey.has(normalized)) {
      nameByPubkey.set(normalized, name);
    }
  }

  return text.replace(NIP27_NPUB_RE, (full, npub: string) => {
    const hex = npubToHex(npub);
    if (!hex) {
      return full;
    }
    const name = nameByPubkey.get(hex);
    return name ? `@${name}` : full;
  });
}
