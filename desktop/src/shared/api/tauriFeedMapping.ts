/**
 * Home-feed wire shapes and their camelCase mapping. Split out of `tauri.ts`
 * so the feed payload can grow without pushing that barrel past the
 * file-size ratchet.
 */

export type RawFeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  created_at: number;
  channel_id: string | null;
  channel_name: string;
  channel_type: string | null;
  tags: string[][];
  category: "mention" | "needs_action" | "activity" | "agent_activity";
  reply_to_self?: boolean;
};

export function fromRawFeedItem(item: RawFeedItem) {
  return {
    id: item.id,
    kind: item.kind,
    pubkey: item.pubkey,
    content: item.content,
    createdAt: item.created_at,
    channelId: item.channel_id,
    channelName: item.channel_name,
    // Canonicalize the wire `null` to undefined so FeedItem's optional
    // channelType contract holds at runtime (enrichment and the DM
    // notification filter both key off `=== undefined`).
    channelType: item.channel_type ?? undefined,
    tags: item.tags,
    category: item.category,
    replyToSelf: item.reply_to_self === true,
  };
}
