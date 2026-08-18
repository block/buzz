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

export type NxtlinqPolicyDraft = {
  name: string;
  version: string;
  scope: string[];
  aud: string[];
  capabilities: Array<
    | {
        type: "filesystem:read";
        include: string[];
        exclude?: string[];
      }
    | {
        type: "filesystem:write";
        include: string[];
        exclude?: string[];
        approvalRequired?: boolean;
      }
    | {
        type: "terminal:execute";
        commands: string[];
        environment?: string[];
        approvalRequired?: boolean;
      }
    | {
        type: "mcp:connect";
        servers: string[];
        approvalRequired?: boolean;
      }
    | {
        type: "mcp:invoke";
        servers: string[];
        tools: string[];
        approvalRequired?: boolean;
      }
  >;
  exp?: number;
};

export type AgentManagementNxtlinqSetupRequest = {
  type: typeof AGENT_MANAGEMENT_REQUEST;
  action: "nxtlinq_setup";
  requestId: string;
  request: {
    channelId: string;
    projectRoot: string;
    explanation: string;
    policy: NxtlinqPolicyDraft;
  };
};

export type AgentManagementRequest =
  | AgentManagementCreateRequest
  | AgentManagementUpdateRequest
  | AgentManagementNxtlinqSetupRequest;

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

function isTextArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.length > 0 && value.every(isText);
}

function isOptionalTextArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(isText);
}

const CAPABILITY_KEYS: Record<string, readonly string[]> = {
  "filesystem:read": ["type", "include", "exclude"],
  "filesystem:write": ["type", "include", "exclude", "approvalRequired"],
  "terminal:execute": ["type", "commands", "environment", "approvalRequired"],
  "mcp:connect": ["type", "servers", "approvalRequired"],
  "mcp:invoke": ["type", "servers", "tools", "approvalRequired"],
};

const INERT_POLICY_SCOPE = "demo:structured-capabilities";
const BUZZ_DEV_MCP_SERVER = "buzz-dev-mcp";
const BUNDLED_DEV_MCP_TOOLS = new Set([
  "read_file",
  "str_replace",
  "shell",
  "buzz_message_send",
  "nxtlinq_setup",
  "todo",
  "_Stop",
  "_PostCompact",
]);
export const REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES = [
  ".env*",
  "**/.env*",
  ".npmrc",
  "**/.npmrc",
  ".netrc",
  "**/.netrc",
  ".pypirc",
  "**/.pypirc",
  ".git-credentials",
  "**/.git-credentials",
  ".git/**",
  "nxtlinq/**",
  ".aws/**",
  "**/.aws/**",
  ".docker/**",
  "**/.docker/**",
  "credentials",
  "**/credentials",
  "**/credentials/**",
  "**/.ssh/**",
  "*.pem",
  "**/*.pem",
  "*.key",
  "**/*.key",
  "*.p12",
  "**/*.p12",
] as const;

/** Conservative owner-editable baseline for setup started directly from UI. */
export function createDefaultNxtlinqPolicyDraft(
  agentName: string,
): NxtlinqPolicyDraft {
  const normalizedName = agentName
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 96);
  return {
    name: normalizedName ? `${normalizedName}-policy` : "buzz-agent-policy",
    version: "1.0.0",
    scope: [INERT_POLICY_SCOPE],
    aud: ["nxtlinq-authorization-gateway"],
    capabilities: [
      {
        type: "filesystem:read",
        include: ["README.md", "package.json", "src/**"],
        exclude: [...REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES],
      },
      {
        type: "mcp:connect",
        servers: [BUZZ_DEV_MCP_SERVER],
      },
    ],
  };
}
const ENVIRONMENT_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES = new Set([
  "BUZZ_PRIVATE_KEY",
  "NOSTR_PRIVATE_KEY",
  "BUZZ_AUTH_TAG",
]);

function hasValidApprovalRequired(capability: Record<string, unknown>) {
  return (
    capability.approvalRequired === undefined ||
    capability.approvalRequired === false
  );
}

function isRelativePolicyPattern(value: string) {
  const normalized = value.replaceAll("\\", "/");
  return (
    !value.includes("\0") &&
    !normalized.startsWith("/") &&
    !/^[A-Za-z]:\//.test(normalized) &&
    !normalized.split("/").includes("..")
  );
}

function isRelativePatternArray(value: unknown): value is string[] {
  return isTextArray(value) && value.every(isRelativePolicyPattern);
}

function isCapability(
  value: unknown,
): value is NxtlinqPolicyDraft["capabilities"][number] {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const capability = value as Record<string, unknown>;
  const allowed =
    typeof capability.type === "string"
      ? CAPABILITY_KEYS[capability.type]
      : undefined;
  if (
    !allowed ||
    !hasOnlyKeys(capability, allowed) ||
    !hasValidApprovalRequired(capability)
  ) {
    return false;
  }

  switch (capability.type) {
    case "filesystem:read":
    case "filesystem:write": {
      if (!isOptionalTextArray(capability.exclude)) return false;
      const excludes = capability.exclude;
      return (
        isRelativePatternArray(capability.include) &&
        excludes.every(isRelativePolicyPattern) &&
        REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES.every((pattern) =>
          excludes.includes(pattern),
        )
      );
    }
    case "terminal:execute":
      return (
        isTextArray(capability.commands) &&
        capability.commands.every((command) => !command.includes("\0")) &&
        isOptionalTextArray(capability.environment) &&
        capability.environment.includes("PATH") &&
        capability.environment.every(
          (name) =>
            ENVIRONMENT_NAME.test(name) &&
            !FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES.has(name),
        )
      );
    case "mcp:connect":
      return isTextArray(capability.servers);
    case "mcp:invoke": {
      if (
        !isTextArray(capability.servers) ||
        capability.servers.length !== 1 ||
        !isTextArray(capability.tools)
      ) {
        return false;
      }
      return !(
        capability.servers.includes(BUZZ_DEV_MCP_SERVER) &&
        capability.tools.some((tool) => BUNDLED_DEV_MCP_TOOLS.has(tool))
      );
    }
    default:
      return false;
  }
}

function isNxtlinqPolicy(value: unknown): boolean {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const policy = value as Record<string, unknown>;
  const capabilities = policy.capabilities;
  const hasRequiredBuzzConnection =
    Array.isArray(capabilities) &&
    capabilities.some((capability) => {
      if (
        typeof capability !== "object" ||
        capability === null ||
        Array.isArray(capability)
      ) {
        return false;
      }
      const candidate = capability as Record<string, unknown>;
      return (
        candidate.type === "mcp:connect" &&
        Array.isArray(candidate.servers) &&
        candidate.servers.includes(BUZZ_DEV_MCP_SERVER)
      );
    });
  const connectedServers = new Set<string>();
  const invokedServers: string[] = [];
  if (Array.isArray(capabilities)) {
    for (const capability of capabilities) {
      if (
        typeof capability !== "object" ||
        capability === null ||
        Array.isArray(capability)
      ) {
        continue;
      }
      const candidate = capability as Record<string, unknown>;
      if (!isTextArray(candidate.servers)) continue;
      if (candidate.type === "mcp:connect") {
        for (const server of candidate.servers) connectedServers.add(server);
      } else if (candidate.type === "mcp:invoke") {
        invokedServers.push(...candidate.servers);
      }
    }
  }
  const invocationConnectionsComplete = invokedServers.every((server) =>
    connectedServers.has(server),
  );
  return (
    hasOnlyKeys(policy, [
      "name",
      "version",
      "scope",
      "aud",
      "capabilities",
      "exp",
    ]) &&
    isText(policy.name) &&
    isText(policy.version) &&
    Array.isArray(policy.scope) &&
    policy.scope.length === 1 &&
    policy.scope[0] === INERT_POLICY_SCOPE &&
    Array.isArray(policy.aud) &&
    policy.aud.length === 1 &&
    policy.aud[0] === "nxtlinq-authorization-gateway" &&
    Array.isArray(capabilities) &&
    capabilities.length > 0 &&
    capabilities.every(isCapability) &&
    hasRequiredBuzzConnection &&
    invocationConnectionsComplete &&
    (policy.exp === undefined ||
      policy.exp === null ||
      (Number.isSafeInteger(policy.exp) && Number(policy.exp) >= 0))
  );
}

/** Parses the strict, policy-only manifest shape accepted by Nxtlinq setup. */
export function parseNxtlinqPolicyDraft(
  value: unknown,
): NxtlinqPolicyDraft | null {
  if (!isNxtlinqPolicy(value)) return null;
  const policy = value as NxtlinqPolicyDraft & { exp?: number | null };
  const { exp, ...policyWithoutExp } = policy;
  return exp === null ? policyWithoutExp : (policy as NxtlinqPolicyDraft);
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
      payload.action !== "nxtlinq_setup") ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;

  if (payload.action === "nxtlinq_setup") {
    if (
      !hasOnlyKeys(request, [
        "channelId",
        "projectRoot",
        "explanation",
        "policy",
      ]) ||
      !isText(request.channelId) ||
      !isText(request.projectRoot) ||
      !isText(request.explanation) ||
      !isNxtlinqPolicy(request.policy)
    ) {
      return null;
    }
    const policy = parseNxtlinqPolicyDraft(request.policy);
    if (!policy) return null;
    return {
      type: AGENT_MANAGEMENT_REQUEST,
      action: "nxtlinq_setup",
      requestId: payload.requestId,
      request: {
        channelId: request.channelId,
        projectRoot: request.projectRoot,
        explanation: request.explanation,
        policy,
      },
    };
  }

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
