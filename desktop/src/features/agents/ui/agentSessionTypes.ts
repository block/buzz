import type { LucideIcon } from "lucide-react";

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
  /**
   * Present on `acp_read` permission frames (kind === "acp_read" + method ===
   * "session/request_permission"). Carries the harness-level permission gate
   * metadata — `requestNonce`, `actionable`, and an optional human-readable
   * `reason`. Payloads are raw ACP; there is no `_buzz` wrapper field.
   */
  authorization?: {
    requestNonce: string;
    actionable: boolean;
    reason?: string;
    /**
     * Wire card-expiry (unix seconds) for an actionable card. Bounds the
     * desktop's retransmit-until-acked loop so a decision published while the
     * harness socket is down is resent until the card expires, never past it.
     * Absent on non-actionable frames and on archived/pre-upgrade frames signed
     * before this field existed.
     */
    expiresAt?: number;
  };
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
      renderClass: "status" | "permission" | "error";
      title: string;
      text: string;
      /** Resolved outcome for permission items (e.g. "Approved (allow_once)", "Denied (reject_once)", "Cancelled"). */
      outcome?: string;
      timestamp: string;
      descriptor?: AgentActivityDescriptor;
      acpSource?: TranscriptAcpSource;
      /**
       * Nonce from the `authorization` envelope on an `acp_read` permission
       * frame. Present only on `renderClass === "permission"` items; used to
       * correlate the `permission_decision` control response and to match
       * incoming `control_result` frames back to this card.
       */
      requestNonce?: string;
      /**
       * Wire card-expiry (unix seconds) from the `authorization` envelope on an
       * actionable `acp_read` permission frame. Bounds the observer-feed card's
       * retransmit-until-acked loop. Absent on read-only cards and on
       * archived/pre-upgrade frames.
       */
      expiresAt?: number;
      /**
       * When `true`, this card is waiting for a user Allow/Deny decision.
       * `false` (or absent) means the card is read-only (auto-handled, or the
       * policy is not `ask`).
       */
      actionable?: boolean;
      /**
       * Human-readable reason string from the `authorization` envelope.
       * Displayed as context below the request description.
       */
      authorizationReason?: string;
      /**
       * Parsed options from the request params, passed back for Allow/Deny
       * button rendering.
       */
      options?: Array<{ optionId: string; kind: string; label?: string }>;
      /**
       * Monotonically increasing token incremented on every authoritative
       * `control_result` delivery failure (`no_active_turn`, `channel_closed`,
       * `no_channel`). The transient `channel_full` status does NOT increment
       * this token — the retransmit orchestrator handles that status
       * automatically. The `PermissionDecisionButtons` component keys its
       * re-enable effect on this value, so a second failure after a retry
       * (same boolean value would not re-trigger the effect) still re-enables
       * the buttons. `undefined` when no failure has occurred.
       */
      deliveryFailed?: number;
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
