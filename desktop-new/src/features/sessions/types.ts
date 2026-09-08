export type Identity = {
  pubkey: string;
  displayName: string;
};

export type Channel = {
  id: string;
  name: string;
  channelType: string;
  visibility: string;
  description: string;
  memberCount: number;
  lastMessageAt?: string | null;
};

export type Participant = {
  pubkey: string;
  role: string;
  isAgent: boolean;
  displayName?: string | null;
};

export type Message = {
  id: string;
  pubkey: string;
  content: string;
  createdAt: number;
  kind: number;
  tags: string[][];
  pending?: "creating" | "sending" | "waiting" | "failed";
  error?: string;
};

export type SessionRecord = {
  channelId: string;
  originChannelId?: string;
  connectedChannelId?: string;
  createdAt: number;
  updatedAt: number;
  incompleteDraft?: string;
};
