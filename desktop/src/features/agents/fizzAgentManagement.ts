import type {
  AgentPersona,
  CreatePersonaInput,
  RespondToMode,
} from "@/shared/api/types";

export const FIZZ_AGENT_MANAGEMENT_REQUEST =
  "fizz_agent_management_request" as const;

export type FizzCreateAgentRequest = {
  type: typeof FIZZ_AGENT_MANAGEMENT_REQUEST;
  action: "create";
  requestId: string;
  request: {
    channelId: string;
    displayName: string;
    systemPrompt: string;
    rationale: string;
    runtime?: string;
    provider?: string;
    model?: string;
    respondTo?: RespondToMode;
  };
};

export type FizzUpdateAgentRequest = {
  type: typeof FIZZ_AGENT_MANAGEMENT_REQUEST;
  action: "update";
  requestId: string;
  request: {
    channelId: string;
    agentName: string;
    rationale: string;
    displayName?: string;
    systemPrompt?: string;
    runtime?: string;
    provider?: string;
    model?: string;
    respondTo?: RespondToMode;
  };
};

export type FizzAgentManagementRequest =
  | FizzCreateAgentRequest
  | FizzUpdateAgentRequest;

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

/** Parses only the deliberately narrow no-secret Fizz request contract. */
export function parseFizzAgentManagementRequest(
  value: unknown,
): FizzAgentManagementRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== FIZZ_AGENT_MANAGEMENT_REQUEST ||
    !isText(payload.requestId) ||
    (payload.action !== "create" && payload.action !== "update") ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;
  if (!isText(request.rationale) || !isRespondTo(request.respondTo))
    return null;

  if (payload.action === "create") {
    if (
      !hasOnlyKeys(request, [
        "channelId",
        "displayName",
        "systemPrompt",
        "rationale",
        "runtime",
        "provider",
        "model",
        "respondTo",
      ])
    ) {
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
      type: FIZZ_AGENT_MANAGEMENT_REQUEST,
      action: "create",
      requestId: payload.requestId,
      request: {
        channelId: request.channelId,
        displayName: request.displayName,
        systemPrompt: request.systemPrompt,
        rationale: request.rationale,
        ...(isText(request.runtime) ? { runtime: request.runtime } : {}),
        ...(isText(request.provider) ? { provider: request.provider } : {}),
        ...(isText(request.model) ? { model: request.model } : {}),
        ...(request.respondTo ? { respondTo: request.respondTo } : {}),
      },
    };
  }

  if (
    !hasOnlyKeys(request, [
      "channelId",
      "agentName",
      "rationale",
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
    type: FIZZ_AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: payload.requestId,
    request: {
      channelId: request.channelId,
      agentName: request.agentName,
      rationale: request.rationale,
      ...changes,
    },
  };
}

export function fizzRequestTargetsEditablePersona(
  persona: AgentPersona | undefined,
): persona is AgentPersona {
  return Boolean(persona && !persona.isBuiltIn && !persona.sourceTeam);
}

export function createInputFromFizzRequest(
  request: Extract<FizzAgentManagementRequest, { action: "create" }>,
): CreatePersonaInput {
  return {
    displayName: request.request.displayName,
    systemPrompt: request.request.systemPrompt,
    runtime: request.request.runtime,
    provider: request.request.provider,
    model: request.request.model,
    behavior: request.request.respondTo
      ? { respondTo: request.request.respondTo }
      : undefined,
  };
}
