import type { AgentPersona } from "@/shared/api/types";
import { hasMention } from "./hasMention";

export type PersonaMentionTarget = {
  displayName: string;
  persona: AgentPersona;
};

export function extractMentionPersonasFromMaps(
  text: string,
  personaMentions: ReadonlyMap<string, string>,
  activePersonaById: ReadonlyMap<string, AgentPersona>,
): PersonaMentionTarget[] {
  const targets: PersonaMentionTarget[] = [];
  const seen = new Set<string>();

  for (const [displayName, personaId] of personaMentions) {
    if (seen.has(personaId) || !hasMention(text, displayName)) continue;
    const persona = activePersonaById.get(personaId);
    if (!persona) continue;
    targets.push({ displayName, persona });
    seen.add(personaId);
  }

  return targets;
}
