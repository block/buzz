import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import {
  managedAgentsQueryKey,
  personasQueryKey,
  relayAgentsQueryKey,
  teamsQueryKey,
} from "@/features/agents/hooks";
import { managedAgentRuntimesQueryKey } from "@/features/agents/managedAgentRuntimeHooks";
import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_AGENT_PROFILE,
  KIND_MANAGED_AGENT,
  KIND_PROFILE,
} from "@/shared/constants/kinds";

// Trailing-coalesce window: a backfill burst (up to 500 inbound events fed
// one-by-one through reconcile) fires one `agents-data-changed` per event.
// Collapsing them into a single invalidate after the burst settles keeps the
// refetch off React Query's implicit in-flight dedup and avoids redundant
// disk-read IPC.
const COALESCE_MS = 200;

// Invalidate the live Agents-tab queries when the backend signals that inbound
// relay events changed the on-disk agents data. Mounted once at the app root
// with empty deps — invalidation is global and has no reason to be
// pubkey-scoped, so it must NOT live inside the pubkey-keyed `usePersonaSync`
// (re-registering per identity switch would leak a listener each time).
export function useAgentsDataRefresh(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let relayAgentTimer: ReturnType<typeof setTimeout> | undefined;
    let relayAgentRetryTimer: ReturnType<typeof setTimeout> | undefined;
    let relayAgentUnsubscribe: (() => Promise<void>) | undefined;
    let cancelled = false;

    const refreshRelayAgents = () => {
      if (relayAgentTimer !== undefined) clearTimeout(relayAgentTimer);
      relayAgentTimer = setTimeout(() => {
        void queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
      }, COALESCE_MS);
    };

    const subscribeToRelayAgentProfiles = (attempt = 0) => {
      if (cancelled) return;
      void relayClient
        .subscribeLive(
          {
            kinds: [KIND_PROFILE, KIND_AGENT_PROFILE, KIND_MANAGED_AGENT],
            limit: 0,
          },
          () => {
            refreshRelayAgents();
          },
        )
        .then((unsubscribe) => {
          if (cancelled) {
            void unsubscribe();
            return;
          }
          relayAgentUnsubscribe = unsubscribe;
        })
        .catch(() => {
          if (cancelled) return;
          const delay = Math.min(1_000 * 2 ** attempt, 30_000);
          relayAgentRetryTimer = setTimeout(
            () => subscribeToRelayAgentProfiles(attempt + 1),
            delay,
          );
        });
    };

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

    subscribeToRelayAgentProfiles();
    const unsubscribeReconnect = relayClient.subscribeToReconnects(() => {
      refreshRelayAgents();
    });

    return () => {
      cancelled = true;
      if (timer !== undefined) clearTimeout(timer);
      if (relayAgentTimer !== undefined) clearTimeout(relayAgentTimer);
      if (relayAgentRetryTimer !== undefined)
        clearTimeout(relayAgentRetryTimer);
      unsubscribeReconnect();
      if (relayAgentUnsubscribe) void relayAgentUnsubscribe();
      void unlisten.then((fn) => fn());
      void unlistenRuntime.then((fn) => fn());
    };
  }, [queryClient]);
}
