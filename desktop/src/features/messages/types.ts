export type TimelineReaction = {
  emoji: string;
  /** Custom (image) emoji URL from the reaction's NIP-30 `emoji` tag, if any. */
  emojiUrl?: string;
  count: number;
  reactedByCurrentUser?: boolean;
  users: Array<{
    pubkey: string;
    displayName: string;
    avatarUrl: string | null;
  }>;
};

export type TimelineMessage = {
  id: string;
  /** Stable local key used to avoid remounting optimistic rows on send ack. */
  renderKey?: string;
  createdAt: number;
  pubkey?: string;
  author: string;
  avatarUrl?: string | null;
  role?: string;
  /** For bot messages, the display name of the persona this bot was created from. */
  personaDisplayName?: string;
  /** For bot messages, the respond-to mode (who can interact with this bot). */
  respondTo?: "owner-only" | "allowlist" | "anyone";
  time: string;
  body: string;
  parentId?: string | null;
  rootId?: string | null;
  depth: number;
  accent?: boolean;
  pending?: boolean;
  edited?: boolean;
  highlighted?: boolean;
  kind?: number;
  tags?: string[][];
  reactions?: TimelineReaction[];
  /**
   * Mirrors {@link RelayEvent.nonContiguous}: merged out-of-band (thread
   * ancestor, thread-panel subtree), so the history around it may be unloaded.
   * The main timeline hides such rows until contiguous paging heals them —
   * otherwise the start of an old day pops in before its middle and end.
   * Thread-panel derivations ignore the flag: a subtree fetched on thread-open
   * is complete within the thread even though it is an island in the timeline.
   */
  nonContiguous?: boolean;
};
