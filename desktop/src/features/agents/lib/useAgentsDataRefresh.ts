import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayAgent, RelayEvent } from "@/shared/api/types";
import { KIND_MANAGED_AGENT } from "@/shared/constants/kinds";
import {
  managedAgentsQueryKey,
  personasQueryKey,
  relayAgentsQueryKey,
  teamsQueryKey,
} from "@/features/agents/hooks";
import { managedAgentRuntimesQueryKey } from "@/features/agents/managedAgentRuntimeHooks";

const COALESCE_MS = 200;
export const RELAY_POLICY_REFRESH_MIN_INTERVAL_MS = 5_000;

export type RelayAgentPolicyCoordinate = {
  agentPubkey: string;
  ownerPubkey: string;
};

function eventDTag(event: RelayEvent): string | null {
  return event.tags.find((tag) => tag[0] === "d")?.[1] ?? null;
}

/**
 * Subscribe only to authenticated managed-agent coordinates already returned by
 * the relay directory. The callback repeats the exact owner+d check because a
 * combined Nostr filter admits the authors×d cross-product.
 */
export function startRelayAgentPolicyRefresh(
  coordinates: RelayAgentPolicyCoordinate[],
  onChange: () => void,
  onError: (error: unknown) => void = (error) => {
    console.warn("Couldn’t subscribe to managed agent policy updates", error);
  },
): () => void {
  if (coordinates.length === 0) return () => {};

  const allowed = new Set(
    coordinates.map(
      ({ ownerPubkey, agentPubkey }) =>
        `${ownerPubkey.toLowerCase()}:${agentPubkey.toLowerCase()}`,
    ),
  );
  const authors = [
    ...new Set(coordinates.map(({ ownerPubkey }) => ownerPubkey)),
  ];
  const agentPubkeys = [
    ...new Set(coordinates.map(({ agentPubkey }) => agentPubkey)),
  ];
  let disposed = false;
  let unsubscribe: (() => Promise<void>) | null = null;
  void relayClient
    .subscribeLive(
      {
        kinds: [KIND_MANAGED_AGENT],
        authors,
        "#d": agentPubkeys,
        limit: 0,
      },
      (event) => {
        const dTag = eventDTag(event);
        if (
          dTag &&
          allowed.has(`${event.pubkey.toLowerCase()}:${dTag.toLowerCase()}`)
        ) {
          onChange();
        }
      },
    )
    .then((nextUnsubscribe) => {
      if (disposed) void nextUnsubscribe();
      else unsubscribe = nextUnsubscribe;
    })
    .catch(onError);

  return () => {
    disposed = true;
    void unsubscribe?.();
  };
}

function relayPolicyCoordinates(agents: RelayAgent[] | undefined) {
  return (agents ?? []).flatMap((agent) =>
    agent.ownerPubkey
      ? [{ agentPubkey: agent.pubkey, ownerPubkey: agent.ownerPubkey }]
      : [],
  );
}

export function useAgentsDataRefresh(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;

    const unlistenRuntime = listen("managed-agent-runtime-status", () => {
      void queryClient.invalidateQueries({
        queryKey: managedAgentRuntimesQueryKey,
      });
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

    let policyStop = () => {};
    let policyTimer: ReturnType<typeof setTimeout> | undefined;
    let policyDirty = false;
    let policyRefreshInFlight = false;
    let policyDisposed = false;
    let coordinateKey = "";

    const refreshPolicyDirectory = () => {
      if (policyRefreshInFlight || policyTimer !== undefined) {
        policyDirty = true;
        return;
      }
      policyRefreshInFlight = true;
      void queryClient
        .invalidateQueries({ queryKey: relayAgentsQueryKey })
        .finally(() => {
          policyRefreshInFlight = false;
          if (policyDisposed) return;
          policyTimer = setTimeout(() => {
            policyTimer = undefined;
            if (policyDirty) {
              policyDirty = false;
              refreshPolicyDirectory();
            }
          }, RELAY_POLICY_REFRESH_MIN_INTERVAL_MS);
        });
    };

    const resubscribePolicy = () => {
      const coordinates = relayPolicyCoordinates(
        queryClient.getQueryData<RelayAgent[]>(relayAgentsQueryKey),
      );
      const nextKey = coordinates
        .map(({ ownerPubkey, agentPubkey }) => `${ownerPubkey}:${agentPubkey}`)
        .sort()
        .join("|");
      if (nextKey === coordinateKey) return;
      coordinateKey = nextKey;
      policyStop();
      policyStop = startRelayAgentPolicyRefresh(
        coordinates,
        refreshPolicyDirectory,
      );
    };
    resubscribePolicy();
    const unsubscribeQueryCache = queryClient
      .getQueryCache()
      .subscribe((event) => {
        if (
          event.query.queryKey.length === relayAgentsQueryKey.length &&
          event.query.queryKey.every(
            (value: unknown, index: number) =>
              value === relayAgentsQueryKey[index],
          )
        ) {
          resubscribePolicy();
        }
      });

    return () => {
      policyDisposed = true;
      if (timer !== undefined) clearTimeout(timer);
      if (policyTimer !== undefined) clearTimeout(policyTimer);
      void unlisten.then((fn) => fn());
      void unlistenRuntime.then((fn) => fn());
      unsubscribeQueryCache();
      policyStop();
    };
  }, [queryClient]);
}
