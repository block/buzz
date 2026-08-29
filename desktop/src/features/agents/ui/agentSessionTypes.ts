import type { LucideIcon } from "lucide-react";
import type { ComputerSessionDescriptor } from "./computerSessionLink";

export type ObserverEvent = {
  seq: number;
  timestamp: string;
  kind: string;
  agentIndex: number | null;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  startedAt?: string | null;
  payload: unknown;
};

export type ConnectionState =
  | "idle"
  | "connecting"
  | "open"
  | "closed"
  | "error";

export type ToolStatus = "executing" | "completed" | "failed" | "pending";

export type AgentActivityRenderClass =
  | "message"
  | "relay-op"
  | "file-edit"
  | "file-read"
  | "skill-read"
  | "image"
  | "shell"
  | "status"
  | "thought"
  | "plan"
  | "permission"
  | "error"
  | "generic"
  | "raw-rail"
  | "suppressed";

export type AgentActivityTone = "read" | "write" | "admin" | "neutral";

export type AgentActivityAction = {
  verb: string;
  object?: string | null;
};

export type AgentActivityDescriptor = {
  renderClass: AgentActivityRenderClass;
  label: string;
  preview: string | null;
  action?: AgentActivityAction;
  tone?: AgentActivityTone;
  operation?: string;
  object?: string | null;
  source?: "mcp" | "shell" | "acp" | "harness" | "fallback";
  groupKey?: string;
  reason?: string;
};

/** Observer/ACP wire label for dev-only transcript debugging. */
export type TranscriptAcpSource = string;

export type TranscriptPermissionRpcId = string | number;

export type TranscriptPermissionBinding = {
  agentPubkey: string;
  relayUrl: string;
  sessionId: string;
  requestId: string;
};

export type TranscriptPermissionToolCall = {
  toolCallId: string;
  title: string;
  kind?: string;
  rawInput?: unknown;
  content?: unknown[];
  locations?: unknown[];
};

export type TranscriptPermissionRequest = {
  binding: TranscriptPermissionBinding;
  rpcId: TranscriptPermissionRpcId;
  expiresAt: string;
  canApproveOnce: boolean;
  detailsComplete: boolean;
  toolCall: TranscriptPermissionToolCall;
};

export type TranscriptPermissionResolution = {
  binding: TranscriptPermissionBinding;
  rpcId: TranscriptPermissionRpcId;
  outcome:
    | "approved"
    | "rejected"
    | "expired"
    | "cancelled"
    | "unavailable"
    | "invalid_request";
  resolvedAt: string;
};

/** Shared optional identity fields attached during transcript construction. */
export type TranscriptItemIdentity = {
  turnId?: string | null;
  sessionId?: string | null;
  channelId?: string | null;
};

export type TranscriptItem =
  | ({
      id: string;
      type: "message";
      renderClass: "message";
      role: "assistant" | "user";
      title: string;
      text: string;
      timestamp: string;
      messageId?: string | null;
      acpSource?: TranscriptAcpSource;
      authorPubkey?: string | null;
    } & TranscriptItemIdentity)
  | ({
      id: string;
      type: "thought";
      renderClass: "thought";
      title: string;
      text: string;
      timestamp: string;
      acpSource?: TranscriptAcpSource;
    } & TranscriptItemIdentity)
  | ({
      id: string;
      type: "plan";
      renderClass: "plan";
      title: string;
      text: string;
      timestamp: string;
      isUpdate?: boolean;
      targetId?: string;
      acpSource?: TranscriptAcpSource;
    } & TranscriptItemIdentity)
  | ({
      id: string;
      type: "lifecycle";
      renderClass: "status" | "error";
      title: string;
      text: string;
      timestamp: string;
      descriptor?: AgentActivityDescriptor;
      acpSource?: TranscriptAcpSource;
    } & TranscriptItemIdentity)
  | ({
      id: string;
      type: "lifecycle";
      renderClass: "permission";
      title: string;
      text: string;
      /** Fixed human-readable terminal label from ACP or semantic resolution telemetry. */
      outcome?: string;
      /** Present only when trusted semantic telemetry supplies the complete binding. */
      permission?: TranscriptPermissionRequest;
      permissionResolution?: TranscriptPermissionResolution;
      timestamp: string;
      descriptor?: AgentActivityDescriptor;
      acpSource?: TranscriptAcpSource;
    } & TranscriptItemIdentity)
  | ({
      id: string;
      type: "metadata";
      renderClass: "raw-rail";
      title: string;
      sections: PromptSection[];
      timestamp: string;
      acpSource?: TranscriptAcpSource;
    } & TranscriptItemIdentity)
  | ({
      id: string;
      type: "tool";
      renderClass: AgentActivityRenderClass;
      descriptor: AgentActivityDescriptor;
      title: string;
      toolName: string;
      buzzToolName: string | null;
      status: ToolStatus;
      args: Record<string, unknown>;
      result: string;
      computerSession?: ComputerSessionDescriptor | null;
      isError: boolean;
      timestamp: string;
      startedAt: string;
      completedAt: string | null;
      acpSource?: TranscriptAcpSource;
    } & TranscriptItemIdentity);

export type PromptSection = {
  title: string;
  body: string;
};

export type BuzzToolInfo = {
  icon: LucideIcon;
  label: string;
  tone: "read" | "write" | "admin";
};
