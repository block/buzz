import type { AgentPersona } from "@/shared/api/types";
import { mentionOccurrences } from "@/shared/lib/mentionOccurrences";

export type PersonaMentionTarget = {
  displayName: string;
  persona: AgentPersona;
};

export function extractMentionPersonasFromMaps(
  text: string,
  personaMentions: ReadonlyMap<string, string>,
  activePersonaById: ReadonlyMap<string, AgentPersona>,
  competingNames: readonly string[] = [],
): PersonaMentionTarget[] {
  const targets: PersonaMentionTarget[] = [];
  const seen = new Set<string>();

  const present = new Set(
    mentionOccurrences(
      text,
      [...personaMentions.keys(), ...competingNames].map((displayName) => ({
        displayName,
      })),
    ).flatMap((match) =>
      match.candidates.map((candidate) => candidate.displayName),
    ),
  );
  for (const [displayName, personaId] of personaMentions) {
    if (seen.has(personaId) || !present.has(displayName)) continue;
    const persona = activePersonaById.get(personaId);
    if (!persona) continue;
    targets.push({ displayName, persona });
    seen.add(personaId);
  }

  return targets;
}
