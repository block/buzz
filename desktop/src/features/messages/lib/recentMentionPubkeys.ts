import type { TimelineMessage } from "@/features/messages/types";
import type { ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Return explicitly addressed pubkeys from the loaded channel window, newest
 * first. DM `p` tags fan out to every participant and cannot distinguish inline
 * mentions, so DMs deliberately fall back to the non-recency ranking ladder.
 * Desktop-authored stream events may include the message author as a structural
 * `p` tag, while SDK-authored events omit it. Filter by author identity rather
 * than tag position so an SDK mention of a reply target remains eligible.
 */
export function getRecentMentionPubkeys(
  messages: readonly TimelineMessage[],
  channelType?: ChannelType | null,
): string[] {
  if (channelType === "dm") return [];

  const seen = new Set<string>();
  const recent: string[] = [];

  for (
    let messageIndex = messages.length - 1;
    messageIndex >= 0;
    messageIndex -= 1
  ) {
    const message = messages[messageIndex];
    const authorPubkey = normalizePubkey(message.pubkey ?? "");
    const tags = message.tags ?? [];
    for (let tagIndex = tags.length - 1; tagIndex >= 0; tagIndex -= 1) {
      const tag = tags[tagIndex];
      if (tag[0] !== "p" || !tag[1]) continue;
      const pubkey = normalizePubkey(tag[1]);
      if (!pubkey || pubkey === authorPubkey || seen.has(pubkey)) continue;
      seen.add(pubkey);
      recent.push(pubkey);
    }
  }

  return recent;
}
