import type { ManagedAgentBackend } from "@/shared/api/types";

/**
 * What a remote agent loses that a local one keeps: its team's standing rules.
 *
 * Local spawn resolves the linked team's instructions and hands them to the
 * harness as `BUZZ_ACP_TEAM_INSTRUCTIONS` (`managed_agents::runtime`). The
 * deploy payload has no team field, so a provider-backed record starts without
 * them — the agent runs, answers, and simply does not know the rules its team
 * was written to enforce.
 *
 * That is observable behaviour, not metadata, so the surfaces that offer or
 * describe a remote agent say it out loud rather than presenting the remote
 * record as equivalent to a local one. Carrying the resolved text to the host
 * is the real fix and a protocol change; until then, disclosure is the honest
 * answer.
 *
 * One owner so the create surface and the edit surface cannot drift into two
 * different accounts of the same limitation.
 */

/** What a remote agent does not receive. Stated unconditionally. */
export const REMOTE_TEAM_INSTRUCTIONS_NOTICE =
  "Agents that run elsewhere do not receive team instructions. A team-linked agent deployed to a host starts without its team's standing rules.";

/** The same fact for a record that HAS a team, where it is already true. */
export const REMOTE_TEAM_INSTRUCTIONS_ACTIVE_NOTICE =
  "This agent runs elsewhere, so it does not receive its team's instructions. Team rules apply to agents running on this computer only.";

/**
 * Whether a record is one the limitation currently bites: linked to a team AND
 * running through a provider.
 *
 * A local team-linked record gets its instructions and needs no notice; a
 * provider-backed record with no team has nothing to lose yet.
 */
export function losesTeamInstructionsRemotely(agent: {
  backend?: ManagedAgentBackend | null;
  teamId?: string | null;
}): boolean {
  return agent.backend?.type === "provider" && Boolean(agent.teamId?.trim());
}
