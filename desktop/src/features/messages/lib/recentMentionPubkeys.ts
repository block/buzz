import type { TimelineMessage } from "@/features/messages/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Return explicitly addressed pubkeys from the loaded channel window, newest
 * first. Replies reserve their first `p` tag for the reply target; top-level
 * messages may carry the author as their first `p` tag, but older/remote event
 * shapes can omit it.
 */
export function getRecentMentionPubkeys(
  messages: readonly TimelineMessage[],
): string[] {
  const seen = new Set<string>();
  const recent: string[] = [];

  for (
    let messageIndex = messages.length - 1;
    messageIndex >= 0;
    messageIndex -= 1
  ) {
    const message = messages[messageIndex];
    const tags = message.tags ?? [];
    const firstPTagIndex = tags.findIndex((tag) => tag[0] === "p");
    const firstPTagPubkey =
      firstPTagIndex >= 0
        ? normalizePubkey(tags[firstPTagIndex]?.[1] ?? "")
        : "";
    const structuralPTagIndex =
      message.parentId != null ||
      firstPTagPubkey === normalizePubkey(message.pubkey ?? "")
        ? firstPTagIndex
        : -1;
    for (
      let tagIndex = tags.length - 1;
      tagIndex > structuralPTagIndex;
      tagIndex -= 1
    ) {
      const tag = tags[tagIndex];
      if (tag[0] !== "p" || !tag[1]) continue;
      const pubkey = normalizePubkey(tag[1]);
      if (!pubkey || seen.has(pubkey)) continue;
      seen.add(pubkey);
      recent.push(pubkey);
    }
  }

  return recent;
}
