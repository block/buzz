import * as React from "react";

import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import {
  resolveProfileActivityAgent,
  type ProfileActivityAgent,
} from "@/features/profile/lib/profileActivityAgent";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";

export function useProfileActivityAgent({
  effectivePubkey,
  isBot,
  managedAgent,
  profile,
  relayAgent,
  viewerIsOwner,
}: {
  effectivePubkey: string | null;
  isBot: boolean;
  managedAgent: ManagedAgent | undefined;
  profile: { avatarUrl?: string | null; displayName?: string | null } | null;
  relayAgent: RelayAgent | undefined;
  viewerIsOwner: boolean;
}): { activityAgent: ProfileActivityAgent | null; viewerCanObserve: boolean } {
  const observableTurns = useActiveAgentTurns(isBot ? effectivePubkey : null);
  const viewerCanObserve = viewerIsOwner || observableTurns.length > 0;
  const activityAgent = React.useMemo(
    () =>
      resolveProfileActivityAgent({
        effectivePubkey,
        isBot,
        managedAgent,
        profile,
        relayAgent,
        viewerCanObserve,
      }),
    [
      effectivePubkey,
      isBot,
      managedAgent,
      profile,
      relayAgent,
      viewerCanObserve,
    ],
  );

  return { activityAgent, viewerCanObserve };
}
