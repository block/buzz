import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type { AgentPersona, AgentTeam, ChannelRole } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";

export type TeamMentionMember = {
  displayName: string;
  kind: "identity" | "persona";
  personaId?: string;
  pubkey?: string;
};

export type PersonaMentionTarget = {
  displayName: string;
  persona: AgentPersona;
};

export type MentionCandidate = {
  kind: "identity" | "persona" | "team";
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  displayName: string | null;
  avatarUrl?: string | null;
  isMember: boolean;
  role?: ChannelRole | null;
  personaName?: string | null;
  secondaryLabel?: string | null;
  ownerPubkey?: string | null;
  isAgent: boolean;
  isManagedAgent?: boolean;
  isGlobalSearchResult?: boolean;
};

export function mentionCandidateLabel(candidate: MentionCandidate) {
  return (
    candidate.displayName ??
    (candidate.pubkey ? truncatePubkey(candidate.pubkey) : "agent")
  );
}

export function globalSearchIdentityKey(candidate: MentionCandidate) {
  if (
    !candidate.isGlobalSearchResult ||
    candidate.isMember ||
    candidate.isAgent
  ) {
    return null;
  }

  const label = candidate.displayName?.trim().toLowerCase();
  if (!label) return null;

  const secondaryLabel = candidate.secondaryLabel?.trim().toLowerCase() ?? "";
  return `global-person:${label}:${secondaryLabel}`;
}

function findTeamMemberTarget(
  persona: AgentPersona,
  candidates: readonly MentionCandidate[],
): TeamMentionMember | null {
  const linked = candidates
    .filter(
      (candidate) =>
        candidate.kind !== "team" && candidate.personaId === persona.id,
    )
    .sort((left, right) => {
      const rank = (candidate: MentionCandidate) => {
        if (candidate.kind === "identity" && candidate.isMember) return 0;
        if (candidate.kind === "identity" && candidate.isManagedAgent) return 1;
        if (candidate.kind === "identity") return 2;
        return 3;
      };
      return rank(left) - rank(right);
    })[0];

  if (linked) {
    return {
      displayName: linked.displayName?.trim() || persona.displayName,
      kind: linked.kind === "identity" ? "identity" : "persona",
      personaId: linked.personaId,
      pubkey: linked.pubkey,
    };
  }

  return persona.isActive
    ? {
        displayName: persona.displayName,
        kind: "persona",
        personaId: persona.id,
      }
    : null;
}

/** Build autocomplete entries for editable, locally owned teams. */
export function buildTeamMentionCandidates(
  teams: readonly AgentTeam[],
  personas: AgentPersona[],
  candidates: readonly MentionCandidate[],
): MentionCandidate[] {
  return teams.flatMap((team) => {
    if (team.isBuiltin || !team.name.trim()) return [];

    const resolution = resolveTeamPersonas(team, personas);
    if (!resolution.isUsable) return [];

    const teamMembers = resolution.resolvedPersonas
      .map((persona) => findTeamMemberTarget(persona, candidates))
      .filter((member): member is TeamMentionMember => member !== null);
    if (teamMembers.length !== resolution.resolvedPersonas.length) return [];

    const mentionNames = new Set<string>();
    for (const member of teamMembers) {
      const mentionName = member.displayName.trim().toLowerCase();
      if (mentionNames.has(mentionName)) return [];
      mentionNames.add(mentionName);
    }

    return [
      {
        kind: "team" as const,
        teamId: team.id,
        teamMembers,
        displayName: team.name.trim(),
        isMember: false,
        isAgent: true,
      },
    ];
  });
}

export function formatTeamMention(
  teamName: string,
  members: readonly TeamMentionMember[],
) {
  return `${teamName}(${members.map((member) => `@${member.displayName}`).join(" ")}) `;
}

export function buildSearchableMentionNames(
  reservedNames: readonly string[],
  candidates: readonly Pick<
    MentionCandidate,
    "displayName" | "personaName" | "secondaryLabel"
  >[],
) {
  const names = [...reservedNames];
  const seen = new Set(names.map((name) => name.toLowerCase()));
  for (const candidate of candidates) {
    for (const name of [
      candidate.displayName,
      candidate.personaName,
      candidate.secondaryLabel,
    ]) {
      const trimmed = name?.trim();
      if (trimmed && !seen.has(trimmed.toLowerCase())) {
        names.push(trimmed);
        seen.add(trimmed.toLowerCase());
      }
    }
  }
  return names;
}
