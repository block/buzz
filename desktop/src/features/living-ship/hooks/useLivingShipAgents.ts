import * as React from "react";

import {
  getAgentWorkingState,
  subscribeAgentWorkingSignal,
} from "@/features/agents/agentWorkingSignal";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  getAgentObserverSnapshot,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import { useChannelsQuery } from "@/features/channels/hooks";
import { LIVING_SHIP_ADVISERS } from "../domain/shipLayout";
import {
  type LivingShipAgentPresentation,
  projectLivingShipAgents,
} from "../domain/shipProjection";

export type LivingShipAgentsState = {
  agents: LivingShipAgentPresentation[];
  isLoading: boolean;
  errorMessage: string | null;
};

export function useLivingShipAgents(): LivingShipAgentsState {
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const [, invalidate] = React.useReducer((value: number) => value + 1, 0);

  React.useEffect(() => {
    const unsubscribeObserver = subscribeAgentObserverStore(invalidate);
    const unsubscribeWorking = subscribeAgentWorkingSignal(invalidate);
    return () => {
      unsubscribeObserver();
      unsubscribeWorking();
    };
  }, []);

  const managedAgents = managedAgentsQuery.data ?? [];
  const commandPubkeys = new Set(
    managedAgents
      .filter((agent) =>
        LIVING_SHIP_ADVISERS.some(
          (visual) => visual.personaId === agent.personaId,
        ),
      )
      .map((agent) => agent.pubkey),
  );
  const workingByPubkey = new Map(
    [...commandPubkeys].map((pubkey) => [pubkey, getAgentWorkingState(pubkey)]),
  );
  const observerEventsByPubkey = new Map(
    [...commandPubkeys].map((pubkey) => [
      pubkey,
      getAgentObserverSnapshot(pubkey, true).events,
    ]),
  );

  return {
    agents: projectLivingShipAgents({
      managedAgents,
      channels: channelsQuery.data ?? [],
      workingByPubkey,
      observerEventsByPubkey,
    }),
    isLoading: managedAgentsQuery.isPending || channelsQuery.isPending,
    errorMessage:
      managedAgentsQuery.error instanceof Error
        ? managedAgentsQuery.error.message
        : channelsQuery.error instanceof Error
          ? channelsQuery.error.message
          : null,
  };
}
