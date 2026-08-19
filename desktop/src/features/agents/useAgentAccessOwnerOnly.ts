/**
 * React hook: report whether this client hides relay agents owned by others.
 *
 * Two inputs: the build flag, and whether the workspace relay advertises the
 * `agent-access-published-policy` NIP-11 extension — a deployment saying it
 * defers to each agent's own published access policy. A marked build honours
 * that, so an operator's decision reaches their users without any of them
 * configuring anything locally.
 *
 * Still one fetch per QueryClient lifetime: the pair is stable for a session
 * on a given relay. Switching workspace relays mid-session therefore keeps the
 * previous answer until the query is invalidated.
 */
import { useQuery } from "@tanstack/react-query";

import { getAgentAccessOwnerOnly } from "@/shared/api/tauriAgentAccess";

export const agentAccessOwnerOnlyQueryKey = [
  "agent-access-owner-only",
] as const;

export function useAgentAccessOwnerOnlyQuery(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: agentAccessOwnerOnlyQueryKey,
    queryFn: () => getAgentAccessOwnerOnly(),
    enabled: options?.enabled ?? true,
    staleTime: Infinity,
    refetchInterval: false,
    retry: false,
  });
}
