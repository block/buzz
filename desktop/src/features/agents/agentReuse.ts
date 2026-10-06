import type {
  AgentPersona,
  CreateManagedAgentInput,
  ManagedAgent,
  ManagedAgentBackend,
} from "@/shared/api/types";

/** Inline normalization — avoids runtime dependency on @/shared/lib/pubkey. */
function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function commandBasename(command: string) {
  const normalized = command.trim().replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] ?? normalized;
}

function normalizeCommandIdentity(command: string) {
  const lower = commandBasename(command).toLowerCase();
  if (lower === "claude-code-acp" || lower === "claude-agent-acp") {
    return "claude-acp";
  }
  return lower;
}

function canonicalJson(value: unknown): string {
  return (
    JSON.stringify(value, (_key, nested) => {
      if (
        nested !== null &&
        typeof nested === "object" &&
        !Array.isArray(nested)
      ) {
        return Object.fromEntries(
          Object.entries(nested).sort(([left], [right]) =>
            left.localeCompare(right),
          ),
        );
      }
      return nested;
    }) ?? "undefined"
  );
}

export function commandsMatch(left: string, right: string) {
  return normalizeCommandIdentity(left) === normalizeCommandIdentity(right);
}

export function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return 0;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? 0 : timestamp;
}

export function pickPreferredManagedAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const leftRunningScore =
      left.status === "running" || left.status === "deployed" ? 1 : 0;
    const rightRunningScore =
      right.status === "running" || right.status === "deployed" ? 1 : 0;
    if (leftRunningScore !== rightRunningScore) {
      return rightRunningScore - leftRunningScore;
    }

    return parseTimestamp(right.updatedAt) - parseTimestamp(left.updatedAt);
  })[0];
}

export function findReusablePersonaAgent(
  agents: ManagedAgent[],
  personaId: string,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  const candidates = findPersonaAgentCandidates(agents, personaId);
  if (candidates.length !== 1) {
    return undefined;
  }
  const inChannel = candidates.filter((agent) =>
    channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  return pickPreferredManagedAgent(
    inChannel.length > 0 ? inChannel : candidates,
  );
}

export function findPersonaAgentCandidates(
  agents: ManagedAgent[],
  personaId: string,
): ManagedAgent[] {
  return agents.filter(
    (agent) =>
      agent.isActive !== false &&
      agent.personaId === personaId &&
      agent.teamId == null,
  );
}

/**
 * Resolve the one instance already deployed for a persona under a specific
 * team. Ambiguous bindings fail closed instead of selecting an arbitrary
 * duplicate. A persona can still have a distinct instance under another team
 * because the team binding is part of the identity lookup.
 */
export function findTeamPersonaAgentCandidates(
  agents: ManagedAgent[],
  personaId: string,
  teamId: string,
): ManagedAgent[] {
  return agents.filter(
    (agent) =>
      agent.isActive !== false &&
      agent.personaId === personaId &&
      agent.teamId === teamId,
  );
}

export function findReusableTeamPersonaAgent(
  agents: ManagedAgent[],
  personaId: string,
  teamId: string,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  const candidates = findTeamPersonaAgentCandidates(agents, personaId, teamId);
  if (candidates.length !== 1) {
    return undefined;
  }
  const inChannel = candidates.filter((agent) =>
    channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  return pickPreferredManagedAgent(
    inChannel.length > 0 ? inChannel : candidates,
  );
}

type ReusableDeploymentInput = {
  runtimeCommand: string;
  model?: string;
  backend?: ManagedAgentBackend;
};

/**
 * Reusing an identity must not silently ignore a new deployment configuration.
 * Access policy is reconciled separately; runtime, backend, and an explicitly
 * requested model must already describe the same deployment.
 */
export function reusableDeploymentConfigurationError(
  agent: ManagedAgent,
  input: ReusableDeploymentInput,
): string | null {
  const requestedBackend = input.backend ?? { type: "local" as const };
  if (agent.backend.type !== requestedBackend.type) {
    return `existing deployment uses ${agent.backend.type}, requested ${requestedBackend.type}`;
  }
  if (
    agent.backend.type === "provider" &&
    requestedBackend.type === "provider" &&
    agent.backend.id !== requestedBackend.id
  ) {
    return `existing deployment uses provider ${agent.backend.id}, requested ${requestedBackend.id}`;
  }
  if (
    agent.backend.type === "provider" &&
    requestedBackend.type === "provider" &&
    canonicalJson(agent.backend.config) !==
      canonicalJson(requestedBackend.config)
  ) {
    return "existing deployment uses different provider configuration";
  }
  if (!commandsMatch(agent.agentCommand, input.runtimeCommand)) {
    return `existing deployment uses runtime ${agent.agentCommand}, requested ${input.runtimeCommand}`;
  }

  const requestedModel = input.model?.trim();
  if (requestedModel && agent.model?.trim() !== requestedModel) {
    return `existing deployment uses model ${agent.model ?? "default"}, requested ${requestedModel}`;
  }
  return null;
}

export function findReusableGenericAgent(
  agents: ManagedAgent[],
  command: string,
  channelMemberPubkeys: ReadonlySet<string>,
): ManagedAgent | undefined {
  const candidates = agents.filter(
    (agent) =>
      !agent.personaId &&
      !agent.systemPrompt?.trim() &&
      commandsMatch(agent.agentCommand, command) &&
      !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  return pickPreferredManagedAgent(candidates);
}

/**
 * Check if a reusable agent exists for the given input. Used by the UI to
 * surface the "reuse vs create new" guardrail before submission.
 */
export function findReusableAgent(
  agents: ManagedAgent[],
  channelMemberPubkeys: ReadonlySet<string>,
  input: {
    personaId?: string | null;
    systemPrompt?: string;
    command: string;
  },
): ManagedAgent | undefined {
  if (input.personaId) {
    return findReusablePersonaAgent(
      agents,
      input.personaId,
      channelMemberPubkeys,
    );
  }
  if (!input.systemPrompt?.trim()) {
    return findReusableGenericAgent(
      agents,
      input.command,
      channelMemberPubkeys,
    );
  }
  return undefined;
}

export function resolveReusableAgentAccessPolicy(
  request: Pick<CreateManagedAgentInput, "respondTo" | "respondToAllowlist">,
  persona?: Pick<AgentPersona, "respondTo" | "respondToAllowlist">,
) {
  const requestedAllowlist = request.respondToAllowlist ?? [];
  if (request.respondTo !== undefined) {
    return {
      respondTo: request.respondTo,
      respondToAllowlist: [...requestedAllowlist],
    };
  }
  if (persona?.respondTo != null) {
    return {
      respondTo: persona.respondTo,
      respondToAllowlist: [
        ...(requestedAllowlist.length > 0
          ? requestedAllowlist
          : persona.respondToAllowlist),
      ],
    };
  }
  return {
    respondTo: "owner-only" as const,
    respondToAllowlist: [...requestedAllowlist],
  };
}
