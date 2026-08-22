/**
 * Home-feed wire types. Split out of `types.ts` so the feed shape can grow
 * without pushing that barrel past the file-size ratchet.
 */

export type FeedItemCategory =
  | "mention"
  | "needs_action"
  | "activity"
  | "agent_activity";

export type FeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  createdAt: number;
  channelId: string | null;
  channelName: string;
  channelType?: string;
  tags: string[][];
  category: FeedItemCategory;
  /**
   * True when this item is in the mention feed only because it replies to one
   * of the user's messages. A reply `p`-tags the author it answers so agent
   * `require_mention` subscriptions receive it, which the mention feed's `#p`
   * query cannot tell apart from a real mention.
   */
  replyToSelf?: boolean;
};

export type HomeFeed = {
  mentions: FeedItem[];
  needsAction: FeedItem[];
  activity: FeedItem[];
  agentActivity: FeedItem[];
};

export type HomeFeedMeta = {
  since: number;
  total: number;
  generatedAt: number;
};

export type HomeFeedResponse = {
  feed: HomeFeed;
  meta: HomeFeedMeta;
};

export type GetHomeFeedInput = {
  since?: number;
  limit?: number;
  types?: string;
};
