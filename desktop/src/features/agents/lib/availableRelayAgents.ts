import { relayAgentIsSharedWithUser } from "./agentAutocompleteEligibility";
import type { RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const STATUS_PRIORITY: Record<RelayAgent["status"], number> = {
  online: 0,
  away: 1,
  offline: 2,
};

/** Relay agents the current identity can actually instruct. */
export function availableRelayAgents(
  relayAgents: readonly RelayAgent[] | undefined,
  sharedChannelIds: ReadonlySet<string>,
  currentPubkey?: string | null,
): RelayAgent[] {
  const seen = new Set<string>();

  return (relayAgents ?? [])
    .filter((agent) => {
      const pubkey = normalizePubkey(agent.pubkey);
      if (seen.has(pubkey)) return false;
      if (!relayAgentIsSharedWithUser(agent, sharedChannelIds, currentPubkey)) {
        return false;
      }
      seen.add(pubkey);
      return true;
    })
    .sort((left, right) => {
      const status =
        STATUS_PRIORITY[left.status] - STATUS_PRIORITY[right.status];
      return status || left.name.localeCompare(right.name);
    });
}
