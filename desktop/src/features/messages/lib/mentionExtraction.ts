import type { AgentPersona } from "@/shared/api/types";
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
  const pubkeys: string[] = [];
  const selectedDisplayNames = new Set(
    [...mentionMap.keys(), ...personaMentionMap.keys()].map((name) =>
      name.trim().toLowerCase(),
    ),
  );

  for (const [displayName, pubkey] of mentionMap) {
    if (hasMention(text, displayName)) pubkeys.push(pubkey);
  }
  for (const candidate of candidates) {
    if (!candidate.pubkey || !candidate.isMember) continue;
    if (pubkeys.includes(candidate.pubkey)) continue;
    const name = candidate.displayName;
    if (name && selectedDisplayNames.has(name.trim().toLowerCase())) continue;
    if (name && hasMention(text, name)) pubkeys.push(candidate.pubkey);
  }

  pubkeys.push(...resolveTeamMentions(text, candidatesWithTeams).pubkeys);
  return [...new Set(pubkeys)];
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
