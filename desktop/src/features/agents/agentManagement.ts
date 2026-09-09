import type {
  AgentPersona,
  CreatePersonaInput,
  RespondToMode,
} from "@/shared/api/types";

export const AGENT_MANAGEMENT_REQUEST = "agent_management_request" as const;

export type AgentManagementCreateRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "create";
  requestId: string;
  request: {
    channelId: string;
    displayName: string;
    systemPrompt: string;
  };
};

export type AgentManagementUpdateRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "update";
  requestId: string;
  request: {
    channelId: string;
    agentName: string;
    displayName?: string;
    systemPrompt?: string;
    runtime?: string;
    provider?: string;
    model?: string;
    respondTo?: RespondToMode;
  };
};

export type AgentManagementRequest =
  | AgentManagementCreateRequest
  | AgentManagementUpdateRequest;

function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isRespondTo(value: unknown): value is RespondToMode | undefined {
  return value === undefined || value === "owner-only" || value === "anyone";
}

function hasOnlyKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
) {
  return Object.keys(value).every((key) => allowed.includes(key));
}

/** Parses only the deliberately narrow no-secret agent-management request contract. */
export function parseAgentManagementRequest(
  value: unknown,
): AgentManagementRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== AGENT_MANAGEMENT_REQUEST ||
    !isText(payload.requestId) ||
    (payload.action !== "create" && payload.action !== "update") ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;

  if (payload.action === "create") {
    if (!hasOnlyKeys(request, ["channelId", "displayName", "systemPrompt"])) {
      return null;
    }
    if (
      !isText(request.channelId) ||
      !isText(request.displayName) ||
      !isText(request.systemPrompt)
    ) {
      return null;
    }
    return {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "create",
      requestId: payload.requestId,
      request: {
        channelId: request.channelId,
        displayName: request.displayName,
        systemPrompt: request.systemPrompt,
      },
    };
  }

  if (
    !isRespondTo(request.respondTo) ||
    !hasOnlyKeys(request, [
      "channelId",
      "agentName",
      "displayName",
      "systemPrompt",
      "runtime",
      "provider",
      "model",
      "respondTo",
    ]) ||
    !isText(request.channelId) ||
    !isText(request.agentName)
  ) {
    return null;
  }
  const changes = {
    ...(isText(request.displayName)
      ? { displayName: request.displayName }
      : {}),
    ...(isText(request.systemPrompt)
      ? { systemPrompt: request.systemPrompt }
      : {}),
    ...(isText(request.runtime) ? { runtime: request.runtime } : {}),
    ...(isText(request.provider) ? { provider: request.provider } : {}),
    ...(isText(request.model) ? { model: request.model } : {}),
    ...(request.respondTo ? { respondTo: request.respondTo } : {}),
  };
  if (Object.keys(changes).length === 0) return null;
  return {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: payload.requestId,
    request: {
      channelId: request.channelId,
      agentName: request.agentName,
      ...changes,
    },
  };
}

export function requestTargetsEditablePersona(
  persona: AgentPersona | undefined,
): persona is AgentPersona {
  return Boolean(persona && !persona.sourceTeam);
}

export function createInputFromRequest(
  request: Extract<AgentManagementRequest, { action: "create" }>,
): CreatePersonaInput {
  return {
    displayName: request.request.displayName,
    systemPrompt: request.request.systemPrompt,
  };
}

/** Definition-field pre-fill (R6 review mode) carried into the merged edit dialog. */
export type AgentReviewOverrides = Partial<{
  displayName: string;
  systemPrompt: string;
  runtime: string | undefined;
  provider: string | undefined;
  model: string | undefined;
  respondTo: string | undefined;
}>;

/**
 * Maps an agent-origin update request to the definition-field overrides the
 * merged edit dialog pre-fills in R6 owner-review mode. Only fields the agent
 * actually requested are carried; an empty result yields `undefined` so the
 * dialog seeds purely from the definition.
 */
export function reviewOverridesForUpdate(
  request: Extract<AgentManagementRequest, { action: "update" }>["request"],
): AgentReviewOverrides | undefined {
  const overrides: AgentReviewOverrides = {};
  if (request.displayName != null) overrides.displayName = request.displayName;
  if (request.systemPrompt != null)
    overrides.systemPrompt = request.systemPrompt;
  if (request.runtime != null) overrides.runtime = request.runtime;
  if (request.provider != null) overrides.provider = request.provider;
  if (request.model != null) overrides.model = request.model;
  // Carry agent-requested respondTo into definition-only review mode so the
  // owner sees and can modify the very access change under review.
  if (request.respondTo != null) overrides.respondTo = request.respondTo;
  return Object.keys(overrides).length > 0 ? overrides : undefined;
}
