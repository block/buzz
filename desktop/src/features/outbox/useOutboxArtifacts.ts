import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import {
  buildOutboxArtifacts,
  OUTBOX_MESSAGE_KINDS,
} from "@/features/outbox/lib/artifacts";
import { relayClient } from "@/shared/api/relayClient";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

const OUTBOX_REFETCH_INTERVAL_MS = 30_000;

export function useOutboxArtifacts() {
  const knownAgentPubkeys = useKnownAgentPubkeys();
  const agentPubkeys = React.useMemo(
    () => [...knownAgentPubkeys].sort(),
    [knownAgentPubkeys],
  );
  const connected = useRelayConnection() === "connected";
  const refetchInterval = useFocusedRefetchInterval(
    connected && agentPubkeys.length > 0 ? OUTBOX_REFETCH_INTERVAL_MS : false,
  );

  return useQuery({
    queryKey: ["outbox-artifacts", ...agentPubkeys],
    enabled: agentPubkeys.length > 0,
    queryFn: async () => {
      const events = await relayClient.fetchEvents({
        authors: agentPubkeys,
        kinds: [...OUTBOX_MESSAGE_KINDS],
        limit: 200,
      });
      return buildOutboxArtifacts(events, new Set(agentPubkeys));
    },
    refetchInterval,
    staleTime: 30_000,
  });
}
