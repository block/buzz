import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { relayClient } from "@/shared/api/relayClient";
import { KIND_MANAGED_AGENT } from "@/shared/constants/kinds";
import {
  managedAgentsQueryKey,
  personasQueryKey,
  relayAgentsQueryKey,
  teamsQueryKey,
} from "@/features/agents/hooks";
import { managedAgentRuntimesQueryKey } from "@/features/agents/managedAgentRuntimeHooks";

// Trailing-coalesce window: a backfill burst (up to 500 inbound events fed
// one-by-one through reconcile) fires one `agents-data-changed` per event.
// Collapsing them into a single invalidate after the burst settles keeps the
// refetch off React Query's implicit in-flight dedup and avoids redundant
// disk-read IPC.
const COALESCE_MS = 200;

/**
 * Subscribe to remote managed-agent policy heads. The directory query remains
 * the trust boundary; this event is only a freshness signal.
 */
export function startRelayAgentPolicyRefresh(
  onChange: () => void,
  onError: (error: unknown) => void = (error) => {
    console.warn("Couldn’t subscribe to managed agent policy updates", error);
  },
): () => void {
  let disposed = false;
  let unsubscribe: (() => Promise<void>) | null = null;
  void relayClient
    .subscribeLive({ kinds: [KIND_MANAGED_AGENT], limit: 0 }, onChange)
    .then((nextUnsubscribe) => {
      if (disposed) {
        void nextUnsubscribe();
      } else {
        unsubscribe = nextUnsubscribe;
      }
    })
    .catch(onError);

  return () => {
    disposed = true;
    void unsubscribe?.();
  };
}

// Invalidate the live Agents-tab queries when the backend signals that inbound
// relay events changed the on-disk agents data. Mounted once at the app root
// with empty deps — invalidation is global and has no reason to be
// pubkey-scoped, so it must NOT live inside the pubkey-keyed `usePersonaSync`
// (re-registering per identity switch would leak a listener each time).
export function useAgentsDataRefresh(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;

    const unlistenRuntime = listen("managed-agent-runtime-status", () => {
      void queryClient.invalidateQueries({
        queryKey: managedAgentRuntimesQueryKey,
      });
      // Pair startup also changes the legacy managed-agent scalar status.
      // Keep that cache synchronized for consumers outside pair-runtime UI.
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    });

    const unlisten = listen("agents-data-changed", () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        void queryClient.invalidateQueries({ queryKey: personasQueryKey });
        void queryClient.invalidateQueries({ queryKey: teamsQueryKey });
        void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
        void queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
      }, COALESCE_MS);
    });

    // Remote owners publish access changes as replacement kind:30177 events.
    // The local owner sync intentionally subscribes only to the current
    // identity's author, so it cannot refresh another owner's autocomplete
    // policy. Watch the managed-policy kind globally and re-read the bounded,
    // trust-checked relay directory when any head changes.
    let relayAgentPolicyTimer: ReturnType<typeof setTimeout> | undefined;
    const stopRelayAgentPolicyRefresh = startRelayAgentPolicyRefresh(() => {
      if (relayAgentPolicyTimer !== undefined) return;
      relayAgentPolicyTimer = setTimeout(() => {
        relayAgentPolicyTimer = undefined;
        void queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
      }, COALESCE_MS);
    });

    return () => {
      if (timer !== undefined) clearTimeout(timer);
      if (relayAgentPolicyTimer !== undefined)
        clearTimeout(relayAgentPolicyTimer);
      void unlisten.then((fn) => fn());
      void unlistenRuntime.then((fn) => fn());
      stopRelayAgentPolicyRefresh();
    };
  }, [queryClient]);
}
