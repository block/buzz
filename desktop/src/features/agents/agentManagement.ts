import type {
  AgentPersona,
  CreatePersonaInput,
  RespondToMode,
  SelfUpdateField,
  UpdatePersonaInput,
} from "@/shared/api/types";

export const AGENT_MANAGEMENT_REQUEST = "agent_management_request" as const;

/** Every self-updatable field, in the order the policy UI and store use. */
export const SELF_UPDATE_FIELDS: readonly SelfUpdateField[] = [
  "system_prompt",
  "model",
  "display_name",
];

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

/** Human label for a self-updatable field, for policy checkboxes and toasts. */
export function selfUpdateFieldLabel(field: SelfUpdateField): string {
  switch (field) {
    case "system_prompt":
      return "system prompt";
    case "model":
      return "model";
    case "display_name":
      return "display name";
  }
}

/** Toggle one field in a policy, keeping the canonical field order. */
export function toggleSelfUpdateField(
  value: readonly SelfUpdateField[],
  field: SelfUpdateField,
  checked: boolean,
): SelfUpdateField[] {
  return SELF_UPDATE_FIELDS.filter((candidate) =>
    candidate === field ? checked : value.includes(candidate),
  );
}

/**
 * The short note shown in the review form when a self-update policy exists
 * but did not fire, so the owner sees why. `null` for the default empty
 * policy: there is nothing to explain when the owner never opted in.
 */
export function selfUpdateReviewNote(outcome: {
  reason: string;
  policy_empty: boolean;
}): string | null {
  if (outcome.policy_empty) return null;
  return `Not applied automatically: ${outcome.reason}. Review and save to apply it.`;
}

/** One line for the audit toast after an agent applied its own update. */
export function describeSelfUpdate(
  displayName: string,
  fields: readonly SelfUpdateField[],
): string {
  const labels = fields.map(selfUpdateFieldLabel);
  const what =
    labels.length === 0
      ? "definition"
      : labels.length === 1
        ? labels[0]
        : `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
  return `${displayName} updated its own ${what}`;
}

export function createInputFromRequest(
  request: Extract<AgentManagementRequest, { action: "create" }>,
): CreatePersonaInput {
  return {
    displayName: request.request.displayName,
    systemPrompt: request.request.systemPrompt,
  };
}

/** Overlay an approved agent-requested edit without resetting unrequested behavior. */
export function updateInputFromRequest(
  request: Extract<AgentManagementRequest, { action: "update" }>,
  current: UpdatePersonaInput,
): UpdatePersonaInput {
  const changes = request.request;
  return {
    ...current,
    displayName: changes.displayName ?? current.displayName,
    systemPrompt: changes.systemPrompt ?? current.systemPrompt,
    runtime: changes.runtime ?? current.runtime,
    provider: changes.provider ?? current.provider,
    model: changes.model ?? current.model,
    ...(changes.respondTo
      ? {
          behavior: {
            respondTo: changes.respondTo,
            respondToAllowlist: [],
            parallelism: current.behavior?.parallelism,
            sessionPolicy: current.behavior?.sessionPolicy,
          },
        }
      : {}),
  };
}
