/**
 * Raw owner-private activity delivered by the runtime adapter. Product
 * projections belong to Agent Activity; this transport shape does not.
 */
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
