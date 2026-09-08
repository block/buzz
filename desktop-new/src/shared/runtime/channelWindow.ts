/** The minimal wire contract for one relay-computed channel-window request. */
export type ChannelWindowCursor = {
  createdAt: number;
  id: string;
};

export type ChannelWindowRequest = {
  channelId: string;
  cursor?: ChannelWindowCursor | null;
};

/** Raw signed event returned by the relay/Tauri channel-window adapter. */
export type RawChannelEvent = {
  id: string;
  pubkey: string;
  content: string;
  created_at: number;
  kind: number;
  tags: string[][];
};
