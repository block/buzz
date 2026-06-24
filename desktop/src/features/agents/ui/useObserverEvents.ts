import * as React from "react";

import {
  ensureRelayObserverSubscription,
  getAgentLeadership,
  getAgentObserverSnapshot,
  getAgentTranscript,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import type { InstanceLeadership } from "@/features/agents/observerRelayStore";
import type { TranscriptItem } from "./agentSessionTypes";

// Stable subscribe reference shared by all useSyncExternalStore hooks.
// subscribeAgentObserverStore already has a fixed identity, so this thin
// wrapper satisfies React's requirement without per-hook useCallback.
const subscribeToStore = (onStoreChange: () => void) =>
  subscribeAgentObserverStore(onStoreChange);

export function useObserverEvents(
  enabled: boolean,
  agentPubkey?: string | null,
) {
  const getSnapshot = React.useCallback(
    () => getAgentObserverSnapshot(agentPubkey, enabled),
    [agentPubkey, enabled],
  );

  const snapshot = React.useSyncExternalStore(subscribeToStore, getSnapshot);

  React.useEffect(() => {
    if (enabled && agentPubkey) {
      void ensureRelayObserverSubscription();
    }
  }, [enabled, agentPubkey]);

  return snapshot;
}

export function useAgentTranscript(
  enabled: boolean,
  agentPubkey?: string | null,
): TranscriptItem[] {
  const getSnapshot = React.useCallback(
    () => getAgentTranscript(agentPubkey, enabled),
    [agentPubkey, enabled],
  );

  return React.useSyncExternalStore(subscribeToStore, getSnapshot);
}

export function useAgentLeadership(
  enabled: boolean,
  agentPubkey?: string | null,
): InstanceLeadership[] {
  const getSnapshot = React.useCallback(
    () => getAgentLeadership(agentPubkey, enabled),
    [agentPubkey, enabled],
  );

  return React.useSyncExternalStore(subscribeToStore, getSnapshot);
}
