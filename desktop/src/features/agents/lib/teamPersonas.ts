import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import type { RelayAgent } from "@/shared/api/types";
import type { TeamCatalogPublication } from "./teamCatalogRelay";

export type ResolvedTeamPersonas = {
  hasMissingPersonas: boolean;
  isComplete: boolean;
  isUsable: boolean;
  missingPersonaCount: number;
  missingPersonaIds: string[];
  resolvedPersonaIds: string[];
  resolvedPersonas: AgentPersona[];
};

export type ResolvedTeamMembers = ResolvedTeamPersonas & {
  hasRemoteMembers: boolean;
  remoteMemberCount: number;
  resolvedRemoteMemberIds: string[];
};

export function emptyResolvedTeamPersonas(): ResolvedTeamPersonas {
  return {
    hasMissingPersonas: false,
    isComplete: true,
    isUsable: false,
    missingPersonaCount: 0,
    missingPersonaIds: [],
    resolvedPersonaIds: [],
    resolvedPersonas: [],
  };
}

export function isResolvedTeamUsable(
  resolution: Pick<ResolvedTeamPersonas, "isComplete" | "resolvedPersonaIds">,
) {
  return resolution.isComplete && resolution.resolvedPersonaIds.length > 0;
}

export function getUsableTeams(
  teams: readonly AgentTeam[],
  personas: AgentPersona[],
) {
  return teams.filter((team) =>
    isResolvedTeamUsable(resolveTeamPersonas(team, personas)),
  );
}

export function resolveTeamPersonas(
  team: Pick<AgentTeam, "personaIds">,
  personas: AgentPersona[],
): ResolvedTeamPersonas {
  const personasById = new Map(
    personas.map((persona) => [persona.id, persona]),
  );
  const resolvedPersonas: AgentPersona[] = [];
  const resolvedPersonaIds: string[] = [];
  const missingPersonaIds: string[] = [];

  for (const personaId of team.personaIds) {
    const persona = personasById.get(personaId);

    if (persona) {
      resolvedPersonas.push(persona);
      resolvedPersonaIds.push(persona.id);
      continue;
    }

    missingPersonaIds.push(personaId);
  }

  const missingPersonaCount = missingPersonaIds.length;

  return {
    hasMissingPersonas: missingPersonaCount > 0,
    isComplete: missingPersonaCount === 0,
    isUsable: missingPersonaCount === 0 && resolvedPersonaIds.length > 0,
    missingPersonaCount,
    missingPersonaIds,
    resolvedPersonaIds,
    resolvedPersonas,
  };
}

function relayIdentityKey(value: string): string {
  return value.trim().toLowerCase();
}

/**
 * Resolve team membership across local definitions and relay-only identities.
 * Remote matches stay outside `resolvedPersonas`, so local deployment gates
 * remain conservative while the UI can explain why a member is not missing.
 */
export function resolveTeamMembers(
  team: Pick<AgentTeam, "id" | "personaIds">,
  personas: readonly AgentPersona[],
  relayAgents: readonly Pick<RelayAgent, "pubkey">[] = [],
  sharedCatalogTeams: readonly Pick<
    TeamCatalogPublication,
    "memberKeys" | "teamDTag" | "ownerPubkey"
  >[] = [],
  catalogOwnerPubkey = "",
): ResolvedTeamMembers {
  const localById = new Map(personas.map((persona) => [persona.id, persona]));
  const relayIdentityIds = new Set<string>();
  for (const agent of relayAgents) {
    const key = relayIdentityKey(agent.pubkey);
    if (key.length > 0) relayIdentityIds.add(key);
  }
  const catalogMemberIds = new Set<string>();
  const normalizedCatalogOwner = relayIdentityKey(catalogOwnerPubkey);
  for (const catalogTeam of sharedCatalogTeams) {
    // The 30178 d-tag is the stable team id, and the active account owns this
    // local team. Require both coordinates before using opaque member keys.
    if (
      catalogTeam.teamDTag !== team.id ||
      relayIdentityKey(catalogTeam.ownerPubkey) !== normalizedCatalogOwner ||
      normalizedCatalogOwner.length === 0
    ) {
      continue;
    }
    for (const memberKey of catalogTeam.memberKeys) {
      const trimmed = memberKey.trim();
      if (trimmed.length > 0) catalogMemberIds.add(trimmed);
    }
  }

  const resolvedPersonas: AgentPersona[] = [];
  const resolvedPersonaIds: string[] = [];
  const resolvedRemoteMemberIds: string[] = [];
  const missingPersonaIds: string[] = [];

  for (const personaId of team.personaIds) {
    const persona = localById.get(personaId);
    if (persona) {
      resolvedPersonas.push(persona);
      resolvedPersonaIds.push(persona.id);
      continue;
    }

    const isRemote =
      relayIdentityIds.has(relayIdentityKey(personaId)) ||
      catalogMemberIds.has(personaId);
    if (isRemote) {
      resolvedRemoteMemberIds.push(personaId);
    } else {
      missingPersonaIds.push(personaId);
    }
  }

  return {
    hasMissingPersonas: missingPersonaIds.length > 0,
    isComplete: missingPersonaIds.length === 0,
    isUsable:
      missingPersonaIds.length === 0 &&
      resolvedRemoteMemberIds.length === 0 &&
      resolvedPersonaIds.length > 0,
    missingPersonaCount: missingPersonaIds.length,
    missingPersonaIds,
    resolvedPersonaIds,
    resolvedPersonas,
    hasRemoteMembers: resolvedRemoteMemberIds.length > 0,
    remoteMemberCount: resolvedRemoteMemberIds.length,
    resolvedRemoteMemberIds,
  };
}
