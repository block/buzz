import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { mergeOwnedAgentPubkeys } from "@/features/agents/knownAgentPubkeys";
import type { UserProfileLookup } from "@/features/profile/lib/identity";

export function useOwnedAgentPubkeys(
  enabled: boolean,
  profiles: UserProfileLookup | undefined,
  currentPubkey: string | undefined,
) {
  const managedAgents = useManagedAgentsQuery({ enabled }).data;
  return React.useMemo(
    () => mergeOwnedAgentPubkeys(managedAgents, profiles, currentPubkey),
    [currentPubkey, managedAgents, profiles],
  );
}
