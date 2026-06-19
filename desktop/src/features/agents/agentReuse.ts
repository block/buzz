import type { ManagedAgent } from "@/shared/api/types";

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
): ManagedAgent | undefined {
  const candidates = agents.filter((agent) => agent.personaId === personaId);
  return pickPreferredManagedAgent(candidates);
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
    return findReusablePersonaAgent(agents, input.personaId);
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
