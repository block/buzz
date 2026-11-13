import type { AgentPersona, AgentTeam } from "@/shared/api/types";

/** Split a search box value into lowercase terms; every term must match. */
function queryTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

/** Case-insensitive AND match of every term against the joined fields. */
function matchesEveryTerm(
  terms: readonly string[],
  fields: ReadonlyArray<string | null | undefined>,
): boolean {
  const haystack = fields
    .filter((field): field is string => Boolean(field))
    .join("\n")
    .toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/**
 * Narrow a persona chooser (the channel "Add agents" dialog) by a search box.
 *
 * Personas match on name, description and system prompt. A team matches on
 * its own name or description, or when any of its member personas matches, so
 * searching for one agent still surfaces the team that would add it.
 */
export function searchPersonaChooser(
  personas: readonly AgentPersona[],
  teams: readonly AgentTeam[],
  query: string,
): { personas: AgentPersona[]; teams: AgentTeam[] } {
  const terms = queryTerms(query);
  if (terms.length === 0) return { personas: [...personas], teams: [...teams] };

  const matchedPersonaIds = new Set(
    personas
      .filter((persona) =>
        matchesEveryTerm(terms, [
          persona.displayName,
          persona.description,
          persona.systemPrompt,
        ]),
      )
      .map((persona) => persona.id),
  );

  return {
    personas: personas.filter((persona) => matchedPersonaIds.has(persona.id)),
    teams: teams.filter(
      (team) =>
        matchesEveryTerm(terms, [team.name, team.description]) ||
        team.personaIds.some((id) => matchedPersonaIds.has(id)),
    ),
  };
}
