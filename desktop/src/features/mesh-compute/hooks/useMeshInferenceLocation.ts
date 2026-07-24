import { useQuery } from "@tanstack/react-query";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { meshAvailability } from "@/shared/api/tauriMesh";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  describeMeshInferenceLocation,
  type MeshInferenceLocation,
} from "../meshInferenceLocation";

/** Serve targets refresh on the relay every 45s; poll a little slower. */
const MESH_AVAILABILITY_REFETCH_MS = 60_000;

export const meshAvailabilityQueryKey = ["mesh-availability"] as const;

/**
 * Where does this agent's inference actually run?
 *
 * Resolves the managed agent record for `agentPubkey`; when its provider is
 * Buzz shared compute (`relay-mesh`), polls live availability and derives a
 * human-readable location ("running Qwen3 8B on 3 nodes on this relay").
 *
 * Returns `null` for non-mesh agents and while availability is still
 * unknown — callers render nothing rather than a guess.
 */
export function useMeshInferenceLocation(
  agentPubkey: string | null,
): MeshInferenceLocation | null {
  const agentsQuery = useManagedAgentsQuery({ enabled: agentPubkey !== null });
  const agent =
    agentPubkey === null
      ? undefined
      : agentsQuery.data?.find(
          (candidate) =>
            normalizePubkey(candidate.pubkey) === normalizePubkey(agentPubkey),
        );
  const isRelayMesh = agent?.provider?.trim() === "relay-mesh";

  const availabilityQuery = useQuery({
    enabled: isRelayMesh,
    queryKey: meshAvailabilityQueryKey,
    queryFn: meshAvailability,
    refetchInterval: MESH_AVAILABILITY_REFETCH_MS,
    staleTime: MESH_AVAILABILITY_REFETCH_MS / 2,
  });

  if (!isRelayMesh || agent === undefined) {
    return null;
  }
  return describeMeshInferenceLocation({
    availability: availabilityQuery.data ?? null,
    model: agent.model,
  });
}
