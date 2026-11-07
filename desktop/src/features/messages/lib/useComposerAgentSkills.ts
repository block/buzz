import * as React from "react";

import {
  getAgentObserverSnapshot,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { extractAvailableAgentSkills } from "./composerAgentSkills";

export function useComposerAgentSkills(
  agentPubkey: string | null,
  channelId: string | null,
) {
  const [revision, setRevision] = React.useState(0);

  React.useEffect(() => {
    if (!agentPubkey) return;
    const normalizedAgentPubkey = normalizePubkey(agentPubkey);
    return subscribeAgentObserverStore((update) => {
      if (
        !update ||
        normalizePubkey(update.agentPubkey) === normalizedAgentPubkey
      ) {
        setRevision((current) => current + 1);
      }
    });
  }, [agentPubkey]);

  if (!agentPubkey) return [];
  void revision;
  return extractAvailableAgentSkills(
    getAgentObserverSnapshot(agentPubkey, true).events,
    channelId,
  );
}
