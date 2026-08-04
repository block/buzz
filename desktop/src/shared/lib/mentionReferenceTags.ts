import { normalizePubkey } from "@/shared/lib/pubkey";
import { MENTION_REFERENCE_TAG } from "@/shared/lib/resolveMentionNames";

function mentionReferenceKey(tag: string[]): string | null {
  if (tag[0] !== MENTION_REFERENCE_TAG || !tag[1]) {
    return null;
  }

  return normalizePubkey(tag[1]);
}

/**
 * Append explicit-mention reference tags for pubkeys that are already being
 * sent as mention targets. Raw `p` tags also carry reply/subscription metadata,
 * so the relay uses these tags to distinguish an intentional @mention from a
 * structural reply-author `p` tag.
 */
export function appendMentionReferenceTags(
  tags: string[][],
  pubkeys: Iterable<string>,
): void {
  const seen = new Set<string>();

  for (const tag of tags) {
    const pubkey = mentionReferenceKey(tag);
    if (pubkey) {
      seen.add(pubkey);
    }
  }

  for (const pubkey of pubkeys) {
    const normalized = normalizePubkey(pubkey);
    if (!normalized || seen.has(normalized)) {
      continue;
    }

    seen.add(normalized);
    tags.push([MENTION_REFERENCE_TAG, normalized]);
  }
}

export function withMentionReferenceTags(
  tags: string[][],
  pubkeys: Iterable<string>,
): string[][] {
  const next = tags.map((tag) => [...tag]);
  appendMentionReferenceTags(next, pubkeys);
  return next;
}
