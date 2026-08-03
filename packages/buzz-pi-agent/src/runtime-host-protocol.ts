import type {
  BuzzSessionEvent,
  ContextSnapshot,
  ModelDescriptor,
  ResourceSnapshot,
  SessionUsageSnapshot,
} from "./types.js";

export interface RuntimeSessionState {
  piSessionId: string;
  sessionFile?: string;
  cwd: string;
  isBusy: boolean;
  models: ModelDescriptor[];
  thinkingLevels: string[];
  context: ContextSnapshot;
  usage: SessionUsageSnapshot;
}

export interface RuntimeHostRequest {
  type: "request";
  id: number;
  method:
    | "create"
    | "prompt"
    | "steer"
    | "abort"
    | "setModel"
    | "setThinkingLevel"
    | "reload"
    | "reset"
    | "replayLifecycle"
    | "ackLifecycle"
    | "dispose"
    | "shutdown";
  sessionId?: string;
  params?: Record<string, unknown>;
}

export interface RuntimeHostResponse {
  type: "response";
  id: number;
  ok: boolean;
  result?: unknown;
  state?: RuntimeSessionState;
  error?: {
    message: string;
    stack?: string;
  };
}

export type RuntimeHostEvent =
  | {
      type: "event";
      sessionId: string;
      eventType: "session_update";
      payload: Record<string, unknown>;
    }
  | {
      type: "event";
      sessionId: string;
      eventType: "buzz_session_event";
      deliveryId?: string;
      payload: BuzzSessionEvent;
    }
  | {
      type: "event";
      sessionId: string;
      eventType: "usage_update";
      payload: {
        usage: SessionUsageSnapshot;
        contextLimit: number;
      };
    };

export type RuntimeHostMessage = RuntimeHostResponse | RuntimeHostEvent;

export interface CreateRuntimeResult {
  resources: ResourceSnapshot;
}
