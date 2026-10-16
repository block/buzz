import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { presenceQueryKey, usePresenceQuery } from "@/features/presence/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { getPresence } from "@/shared/api/tauri";
import type { PresenceLookup, PresenceStatus } from "@/shared/api/types";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Availability is relay presence, never a retained deployment receipt or PID. */
export function resolveAgentAvailability(
  status: PresenceStatus | undefined,
  presenceLoaded: boolean,
  connected: boolean,
): PresenceStatus | undefined {
  // Missing entries in a successful presence snapshot mean offline. Failed or
  // disconnected reads cannot establish availability (including cached online).
  return presenceLoaded && connected ? (status ?? "offline") : undefined;
}

/**
 * One-shot availability for an action-time decision made outside a rendered
 * surface (no presence subscription on that path). A failed read is unknown,
 * never offline: unknown keeps trusting the lifecycle receipt.
 */
export async function fetchAgentAvailability(
  pubkey: string,
): Promise<PresenceStatus | undefined> {
  return (await fetchAgentsAvailability([pubkey])).get(normalizePubkey(pubkey));
}

/** Batched {@link fetchAgentAvailability}: one presence read for many agents,
 * keyed by normalized pubkey. A failed read leaves every entry unknown. */
export async function fetchAgentsAvailability(
  pubkeys: readonly string[],
): Promise<Map<string, PresenceStatus | undefined>> {
  const normalized = [...new Set(pubkeys.map(normalizePubkey))];
  const availability = new Map<string, PresenceStatus | undefined>();
  if (normalized.length === 0) return availability;
  try {
    const lookup = await getPresence(normalized);
    const connected = relayClient.getConnectionState() === "connected";
    for (const pubkey of normalized) {
      availability.set(
        pubkey,
        resolveAgentAvailability(lookup[pubkey], true, connected),
      );
    }
  } catch {
    // Unknown, never offline: callers keep trusting the lifecycle receipt.
  }
  return availability;
}

/** Positive presence blocks another start, but never grants lifecycle control.
 * Missing/offline presence is not proof that starting another body is safe.
 */
export function agentPresenceStartBlockReason(
  isLifecycleActive: boolean,
  availability: PresenceStatus | undefined,
): string | undefined {
  return !isLifecycleActive &&
    (availability === "online" || availability === "away")
    ? "This agent is present on the relay. Starting another instance is unavailable."
    : undefined;
}

/** Read availability from the surface-owned snapshot, not per-row polling. */
export type AgentAvailabilityReader = (
  pubkey: string | null | undefined,
) => PresenceStatus | undefined;

/** One query/connection observer for a surface's cards and lifecycle actions. */
export function useAgentAvailabilityLookup(
  pubkeys: string[],
  options?: { enabled?: boolean },
) {
  const query = usePresenceQuery(pubkeys, options);
  const queryClient = useQueryClient();
  // Subscribe for display updates; action-time reads below must also see a
  // disconnect/error that occurred while an action awaited channel discovery.
  const connection = useRelayConnection({ degradedAfterMs: 0 });
  const keyId = JSON.stringify(presenceQueryKey(pubkeys));
  // These dependencies subscribe the render to data/status changes, while the
  // callback reads the cache at invocation time (including after an await).
  // biome-ignore lint/correctness/useExhaustiveDependencies: Track observer changes and invalidate memoized consumers; invocation must read live state, not capture those values.
  const getAvailability: AgentAvailabilityReader = React.useMemo(() => {
    const key: string[] = JSON.parse(keyId);
    const requested = new Set(key.slice(1));
    return (pubkey) => {
      const normalized = pubkey ? normalizePubkey(pubkey) : "";
      // A successful subset says nothing about unqueried persona siblings.
      if (!normalized || !requested.has(normalized)) return undefined;
      const state = queryClient.getQueryState<PresenceLookup>(key);
      return resolveAgentAvailability(
        state?.data?.[normalized],
        state?.status === "success",
        relayClient.getConnectionState() === "connected",
      );
    };
  }, [keyId, queryClient, query.data, query.isSuccess, connection]);
  return { query, getAvailability };
}

/** Single-identity surfaces use the same authority as aggregate surfaces. */
export function useAgentAvailability(pubkey: string | null | undefined) {
  const { query, getAvailability } = useAgentAvailabilityLookup(
    pubkey ? [pubkey] : [],
  );
  return { query, status: getAvailability(pubkey) };
}
