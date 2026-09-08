/**
 * React hook: report whether this build forces owner-only agent access.
 *
 * The value is baked at build time, so it cannot change while the app runs.
 * The query key is stable and the result never goes stale, which keeps one
 * fetch per QueryClient lifetime and gives every caller the same answer.
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
