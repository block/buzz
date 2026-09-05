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

export type ActivityStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "needs_you"
  | "unavailable";

export type ActivityItem = {
  id: string;
  label: string;
  detail?: string;
  status: ActivityStatus;
  timestamp: string;
};

export type AgentTurn = {
  key: string;
  agentPubkey: string;
  agentName: string;
  sessionId: string;
  turnId: string;
  status: ActivityStatus;
  items: ActivityItem[];
};

export type ObserverEvent = {
  seq: number;
  timestamp: string;
  kind: string;
  channelId?: string;
  sessionId?: string;
  turnId?: string;
  agentPubkey?: string;
  payload: Record<string, unknown>;
};
