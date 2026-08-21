import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Returns a `Set<string>` of persona IDs whose managed agents are already
 * members of the given channel. The query is only enabled when `enabled` is
 * true (e.g. when the dialog is open).
 */
export function useInChannelPersonaIds(
  channelId: string | null,
  enabled: boolean,
): ReadonlySet<string> {
  const membersQuery = useChannelMembersQuery(channelId, enabled);
  const managedAgentsQuery = useManagedAgentsQuery();

  return React.useMemo(() => {
    const members = membersQuery.data;
    const managedAgents = managedAgentsQuery.data;
    if (!members || !managedAgents) {
      return new Set<string>();
    }

    const memberPubkeys = new Set(
      members.map((m) => normalizePubkey(m.pubkey)),
    );

    const ids = new Set<string>();
    for (const agent of managedAgents) {
      if (agent.personaId && memberPubkeys.has(normalizePubkey(agent.pubkey))) {
        ids.add(agent.personaId);
      }
    }
    return ids;
  }, [membersQuery.data, managedAgentsQuery.data]);
}
