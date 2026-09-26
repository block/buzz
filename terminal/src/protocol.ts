/** Signed, host-verified Nostr event. Key material never crosses this boundary. */
export interface RelayEvent {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  content: string;
  tags: string[][];
  sig: string;
}

/** A persistent, bounded relay subscription. Explicit kinds are mandatory. */
export interface Filter {
  kinds: number[];
  authors?: string[];
  ids?: string[];
  since?: number;
  until?: number;
  limit?: number;
  "#h"?: string[];
  "#d"?: string[];
  "#p"?: string[];
  "#e"?: string[];
}

/** Immutable send target; retries reuse requestKey and exactly the same payload. */
export interface MessageRequest {
  requestKey: string;
  channelId: string;
  content: string;
  recipientPubkeys: string[];
  rootEventId?: string;
}

/** Protocol v1 requests, encoded as one JSON object per line on host stdin. */
export interface Requests {
  subscribe: { subscriptionId: string; filters: Filter[] };
  unsubscribe: { subscriptionId: string };
  sendMessage: MessageRequest;
  createChannel: { requestKey: string; channelId: string };
  joinChannel: { requestKey: string; channelId: string };
  addMember: { requestKey: string; channelId: string; pubkey: string };
  reconnect: Record<string, never>;
}

/** Responses and unsolicited notifications from the local Rust host. */
export type HostMessage =
  | { id: string; result: unknown }
  | { id: string; error: { code: string; message: string; eventId?: string } }
  | {
      type: "ready";
      protocolVersion: 1;
      pubkey: string;
      relayPubkey: string;
      relayUrl: string;
    }
  | {
      type: "connection";
      status: "connecting" | "connected" | "disconnected" | "failed";
      message?: string;
    }
  | {
      type: "event";
      subscriptionId: string;
      event: RelayEvent;
      ownerPubkey?: string;
    }
  | { type: "eose"; subscriptionId: string }
  | { type: "closed"; subscriptionId: string; message: string }
  | { type: "notice"; message: string }
  | {
      type: "observer";
      eventId: string;
      agentPubkey: string;
      envelope: ObserverEnvelope;
    };

/** Best-effort ACP telemetry; never a durable execution history. */
export interface ObserverEnvelope {
  seq: number;
  timestamp: string;
  kind: string;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  startedAt?: string;
  payload: unknown;
}
