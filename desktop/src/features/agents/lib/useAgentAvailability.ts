import { usePresenceQuery } from "@/features/presence/hooks";
import type { PresenceStatus } from "@/shared/api/types";
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

/** Share the existing presence query/subscription; no separate status cache. */
export function useAgentAvailability(pubkey: string | null | undefined) {
  const query = usePresenceQuery(pubkey ? [pubkey] : []);
  const connection = useRelayConnection();
  const status = resolveAgentAvailability(
    pubkey ? query.data?.[normalizePubkey(pubkey)] : undefined,
    query.isSuccess,
    connection === "connected",
  );
  return { query, status };
}
