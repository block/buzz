import type { AgentPersona } from "@/shared/api/types";
import { extractMentionPubkeys as extractDirectMentionPubkeys } from "./extractMentionPubkeys";
import { hasMention } from "./hasMention";
import {
  type MentionCandidate,
  resolveTeamMentions,
} from "./mentionCandidates";

export type PersonaMentionTarget = {
  displayName: string;
  persona: AgentPersona;
};

export function extractMentionPubkeysFromCandidates({
  candidates,
  candidatesWithTeams,
  mentionMap,
  personaMentionMap,
  text,
}: {
  candidates: readonly MentionCandidate[];
  candidatesWithTeams: readonly MentionCandidate[];
  mentionMap: ReadonlyMap<string, string>;
  personaMentionMap: ReadonlyMap<string, string>;
  text: string;
}): string[] {
  return [
    ...extractDirectMentionPubkeys({
      text,
      selectedMentions: mentionMap,
      selectedDisplayNames: personaMentionMap.keys(),
      memberCandidates: candidates,
    }),
    ...resolveTeamMentions(text, candidatesWithTeams).pubkeys,
  ];
}

export function extractMentionPersonasFromCandidates({
  activePersonaById,
  candidatesWithTeams,
  personaMentionMap,
  text,
}: {
  activePersonaById: ReadonlyMap<string, AgentPersona>;
  candidatesWithTeams: readonly MentionCandidate[];
  personaMentionMap: ReadonlyMap<string, string>;
  text: string;
}): PersonaMentionTarget[] {
  const targets: PersonaMentionTarget[] = [];
  const seen = new Set<string>();
  const addTarget = (displayName: string, personaId: string) => {
    if (seen.has(personaId)) return;
    const persona = activePersonaById.get(personaId);
    if (!persona) return;
    targets.push({ displayName, persona });
    seen.add(personaId);
  };

  for (const [displayName, personaId] of personaMentionMap) {
    if (hasMention(text, displayName)) addTarget(displayName, personaId);
  }
  for (const member of resolveTeamMentions(text, candidatesWithTeams)
    .personaMembers) {
    if (member.personaId) addTarget(member.displayName, member.personaId);
  }
  return targets;
}
