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

export type AgentManagementSpawnTempRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "spawn_temp";
  requestId: string;
  request: {
    channelId: string;
    displayName: string;
    systemPrompt: string;
    ttlSeconds?: number;
  };
};

export type AgentManagementDestroyTempRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "destroy_temp";
  requestId: string;
  request: {
    channelId: string;
    agentName: string;
  };
};

export type AgentManagementRequest =
  | AgentManagementCreateRequest
  | AgentManagementUpdateRequest
  | AgentManagementSpawnTempRequest
  | AgentManagementDestroyTempRequest;

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

function isPositiveInt(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1;
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
    (payload.action !== "create" &&
      payload.action !== "update" &&
      payload.action !== "spawn_temp" &&
      payload.action !== "destroy_temp") ||
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

  if (payload.action === "spawn_temp") {
    if (
      !hasOnlyKeys(request, [
        "channelId",
        "displayName",
        "systemPrompt",
        "ttlSeconds",
      ]) ||
      !isText(request.channelId) ||
      !isText(request.displayName) ||
      !isText(request.systemPrompt) ||
      (request.ttlSeconds !== undefined && !isPositiveInt(request.ttlSeconds))
    ) {
      return null;
    }
    return {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "spawn_temp",
      requestId: payload.requestId,
      request: {
        channelId: request.channelId,
        displayName: request.displayName,
        systemPrompt: request.systemPrompt,
        ...(request.ttlSeconds !== undefined
          ? { ttlSeconds: request.ttlSeconds }
          : {}),
      },
    };
  }

  if (payload.action === "destroy_temp") {
    if (
      !hasOnlyKeys(request, ["channelId", "agentName"]) ||
      !isText(request.channelId) ||
      !isText(request.agentName)
    ) {
      return null;
    }
    return {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "destroy_temp",
      requestId: payload.requestId,
      request: {
        channelId: request.channelId,
        agentName: request.agentName,
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
