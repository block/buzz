import type {
  AgentPersona,
  ManagedAgent,
  RespondToMode,
  UpdateManagedAgentInput,
} from "@/shared/api/types";

const DEFAULT_RESPOND_TO: RespondToMode = "owner-only";
const DEFAULT_PARALLELISM = 10;

/**
 * Build the instance patch for an explicitly edited persona behavior group.
 *
 * Definition behavior is a mint-time default, while existing instances can
 * carry intentional overrides. The selected linked instance is therefore
 * updated only when the old and new definition behavior differs; unrelated
 * definition edits must not overwrite its gate or parallelism.
 */
export function personaBehaviorManagedAgentPatch(
  previousPersona: AgentPersona | undefined,
  persona: AgentPersona,
): Pick<
  UpdateManagedAgentInput,
  "parallelism" | "respondTo" | "respondToAllowlist"
> | null {
  if (
    previousPersona === undefined ||
    (previousPersona.respondTo === persona.respondTo &&
      stringArrayEqual(
        previousPersona.respondToAllowlist ?? [],
        persona.respondToAllowlist ?? [],
      ) &&
      previousPersona.parallelism === persona.parallelism)
  ) {
    return null;
  }

  return {
    respondTo: persona.respondTo ?? DEFAULT_RESPOND_TO,
    respondToAllowlist: [...(persona.respondToAllowlist ?? [])],
    parallelism: persona.parallelism ?? DEFAULT_PARALLELISM,
  };
}

export function linkedInstanceForPersonaRequest(
  agents: readonly ManagedAgent[],
  personaId: string,
  agentName: string,
  requestingPubkey?: string | null,
): ManagedAgent | null {
  const targetName = agentName.trim().toLocaleLowerCase();
  const candidates = agents.filter(
    (agent) =>
      agent.personaId === personaId &&
      agent.name.trim().toLocaleLowerCase() === targetName,
  );
  const normalizedRequestingPubkey = requestingPubkey?.trim().toLowerCase();
  const requestingInstance = normalizedRequestingPubkey
    ? candidates.find(
        (agent) => agent.pubkey.toLowerCase() === normalizedRequestingPubkey,
      )
    : undefined;
  if (requestingInstance) return requestingInstance;

  // A definition can mint multiple independently overridden instances. Never
  // fan a definition edit out when its display name does not identify one.
  return candidates.length === 1 ? candidates[0] : null;
}

function stringArrayEqual(left: readonly string[], right: readonly string[]) {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}
