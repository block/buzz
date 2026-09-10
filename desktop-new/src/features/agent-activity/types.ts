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

/** Owner-private activity from an agent turn. Not a shared conversation event. */
export type AgentTurn = {
  key: string;
  agentPubkey: string;
  agentName: string;
  sessionId: string;
  turnId: string;
  channelId: string;
  status: ActivityStatus;
  items: ActivityItem[];
};
