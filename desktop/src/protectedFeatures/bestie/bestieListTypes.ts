/** Kind of Bestie list item shown in the Bestie DM RHS. */
export type BestieListKind = "todo" | "reminder";

export type BestieListItem = {
  createdAt: number;
  /** Unix seconds; reminders only. */
  dueAt: number | null;
  id: string;
  kind: BestieListKind;
  /** Message id that created this item (agent-add path), when known. */
  sourceMessageId: string | null;
  status: "open" | "done";
  text: string;
  updatedAt: number;
};

export type BestieListState = {
  items: BestieListItem[];
  /** Message ids already applied so reloads don't duplicate agent-add actions. */
  processedMessageIds: string[];
  version: 1;
};

export type BestieListScope = {
  agentPubkey: string;
  ownerPubkey: string;
  relayUrl: string;
};

export type BestieListAddInput = {
  dueAt?: number | null;
  kind: BestieListKind;
  sourceMessageId?: string | null;
  text: string;
};
