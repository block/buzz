//! Circuit-status React hooks, split out of `observerRelayStore.ts`.
//!
//! Merging main pushed that file past the repo's 1000-line budget, and an
//! over-budget file may not grow. These two hooks are the cleanest seam: they
//! read circuit state from `agentCircuitStatus` and use the store only for its
//! subscribe function, so nothing module-private moves with them.
//!
//! They live here rather than in `agentCircuitStatus.ts` because that module is
//! imported by `observerRelayStore`; depending on the store from there would
//! close an import cycle.

import * as React from "react";

import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  getAgentCircuitStatus,
  getOpenCircuitPubkeySignature,
  type AgentCircuitStatus,
} from "./agentCircuitStatus";
import { subscribeAgentObserverStore } from "./observerRelayStore";

/** Persistent per-agent circuit-breaker status, independent of channel/transcript scope. */
export function useAgentCircuitStatus(
  agentPubkey: string | null | undefined,
): AgentCircuitStatus {
  return React.useSyncExternalStore(subscribeAgentObserverStore, () =>
    getAgentCircuitStatus(agentPubkey),
  );
}

/**
 * The subset of `agents` whose circuit is currently open. Generic over the
 * caller's own agent shape so it works with any `{ pubkey: string }`-shaped
 * roster (e.g. a channel's bot list) without this module needing to know
 * about it. The `useSyncExternalStore` snapshot is a primitive signature
 * string (see `getOpenCircuitPubkeySignature`) rather than an array, so it's
 * reference-stable across renders where nothing changed without this module
 * needing a cache; the actual `T[]` is then derived per-render via a normal
 * `useMemo` keyed on both `agents` and that signature.
 */
export function useOpenCircuitAgents<T extends { pubkey: string }>(
  agents: readonly T[],
): T[] {
  const signature = React.useSyncExternalStore(
    subscribeAgentObserverStore,
    () => getOpenCircuitPubkeySignature(agents.map((agent) => agent.pubkey)),
  );
  return React.useMemo(() => {
    if (!signature) return [];
    const openPubkeys = new Set(signature.split(","));
    return agents.filter((agent) =>
      openPubkeys.has(normalizePubkey(agent.pubkey)),
    );
  }, [agents, signature]);
}
