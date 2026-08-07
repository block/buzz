import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type { AgentPersona, AgentTeam, ChannelRole } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { TEAM_MENTION_TAG } from "@/shared/lib/resolveMentionNames";
import { hasMention } from "./hasMention";

export type TeamMentionMember = {
  avatarUrl?: string | null;
  displayName: string;
  kind: "identity" | "persona";
  personaId?: string;
  pubkey?: string;
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
      avatarUrl: linked.avatarUrl ?? persona.avatarUrl,
      displayName: linked.displayName?.trim() || persona.displayName,
      kind: linked.kind === "identity" ? "identity" : "persona",
      personaId: linked.personaId,
      pubkey: linked.pubkey,
    };
  }

  return persona.isActive
    ? {
        avatarUrl: persona.avatarUrl,
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
  _members?: readonly TeamMentionMember[],
) {
  return `@${teamName} `;
}

export function resolveTeamMentions(
  text: string,
  candidates: readonly MentionCandidate[],
): {
  personaMembers: TeamMentionMember[];
  pubkeys: string[];
  tags: string[][];
} {
  const personaMembers: TeamMentionMember[] = [];
  const pubkeys = new Set<string>();
  const seenPersonaIds = new Set<string>();
  const tags: string[][] = [];

  for (const candidate of candidates) {
    if (
      candidate.kind !== "team" ||
      !candidate.teamId ||
      !candidate.displayName ||
      !hasMention(text, candidate.displayName)
    ) {
      continue;
    }

    tags.push([TEAM_MENTION_TAG, candidate.teamId, candidate.displayName]);
    for (const member of candidate.teamMembers ?? []) {
      if (member.pubkey) {
        pubkeys.add(member.pubkey);
      } else if (
        member.kind === "persona" &&
        member.personaId &&
        !seenPersonaIds.has(member.personaId)
      ) {
        personaMembers.push(member);
        seenPersonaIds.add(member.personaId);
      }
    }
  }

  return { personaMembers, pubkeys: [...pubkeys], tags };
}
