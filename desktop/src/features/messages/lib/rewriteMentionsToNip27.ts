import { getMentionOffset } from "@/features/messages/lib/hasMention";
import { safeNpub } from "@/shared/lib/nostrUtils";

/**
 * Rewrite selected `@displayName` mentions in `text` to NIP-27 `nostr:npub1…`
 * URIs for the wire event. Composer UX can keep typing `@name`; callers should
 * rewrite only at send/edit time.
 *
 * Longer display names are applied first so multi-word mentions win over
 * shorter prefixes. Code spans/blocks are skipped via `getMentionOffset`.
 */
export function rewriteMentionsToNip27(
  text: string,
  nameToPubkey: Iterable<readonly [string, string]>,
): string {
  if (!text.includes("@")) {
    return text;
  }

  const entries = [...nameToPubkey]
    .map(([name, pubkey]) => [name.trim(), pubkey.trim()] as const)
    .filter(([name, pubkey]) => name.length > 0 && pubkey.length > 0)
    .sort((a, b) => b[0].length - a[0].length);

  if (entries.length === 0) {
    return text;
  }

  let result = text;
  for (const [name, pubkey] of entries) {
    const npub = safeNpub(pubkey);
    if (!npub) {
      continue;
    }
    const uri = `nostr:${npub}`;
    const tokenLength = 1 + name.length;

    // Replace every non-code occurrence. Offset is recomputed each pass
    // because earlier replacements shift later positions.
    for (;;) {
      const offset = getMentionOffset(result, name);
      if (offset === null) {
        break;
      }
      result =
        result.slice(0, offset) + uri + result.slice(offset + tokenLength);
    }
  }

  return result;
}
