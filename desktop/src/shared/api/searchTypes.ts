export type SearchMessagesInput = {
  q: string;
  limit?: number;
  channelId?: string;
  /** Hex pubkeys for `from:` operator. */
  authors?: string[];
  /** Unix seconds (`after:YYYY-MM-DD`). */
  since?: number;
  /** Unix seconds (`before:YYYY-MM-DD`). */
  until?: number;
};

export type SearchHit = {
  eventId: string;
  content: string;
  kind: number;
  pubkey: string;
  channelId: string | null;
  channelName: string | null;
  createdAt: number;
  score: number;
  threadRootId?: string | null;
};

export type SearchMessagesResponse = {
  hits: SearchHit[];
  found: number;
};
