import type { AdviserId } from "./briefContracts";

export interface CommandTeamPersona {
  readonly adviser: AdviserId;
  readonly personaId: string;
  readonly label: string;
  readonly detail: string;
}

export const COMMAND_TEAM_PERSONAS = Object.freeze([
  {
    adviser: "chief_of_staff",
    personaId: "builtin:command-chief-of-staff",
    label: "Chief of Staff",
    detail: "Consolidates the command brief",
  },
  {
    adviser: "operations",
    personaId: "builtin:command-operations",
    label: "Operations",
    detail: "Priorities, readiness and risk",
  },
  {
    adviser: "intelligence",
    personaId: "builtin:command-intelligence",
    label: "Maritime N2",
    detail: "Regional intelligence, threats and warning",
  },
  {
    adviser: "logistics",
    personaId: "builtin:command-logistics",
    label: "Logistics",
    detail: "Replenishment, sustainment and dependencies",
  },
  {
    adviser: "navigation",
    personaId: "builtin:command-navigation",
    label: "Navigation",
    detail: "Evidence and source limitations",
  },
  {
    adviser: "daily_routine",
    personaId: "builtin:command-daily-routine",
    label: "Daily Routine",
    detail: "Calendar, reminders and routine",
  },
  {
    adviser: "reporting",
    personaId: "builtin:command-reporting",
    label: "Reporting",
    detail: "Reports, returns and missing inputs",
  },
  {
    adviser: "plans",
    personaId: "builtin:command-plans",
    label: "Plans",
    detail: "30, 60 and 90-day outlook",
  },
] as const satisfies readonly CommandTeamPersona[]);

const COMMAND_TEAM_BY_PERSONA: ReadonlyMap<string, CommandTeamPersona> =
  new Map(
    COMMAND_TEAM_PERSONAS.map(
      (persona) => [persona.personaId, persona] as const,
    ),
  );

export function isCommandTeamPersonaId(id: string): boolean {
  return COMMAND_TEAM_BY_PERSONA.has(id);
}

export function commandAdviserForPersona(id: string): AdviserId | undefined {
  return COMMAND_TEAM_BY_PERSONA.get(id)?.adviser;
}
