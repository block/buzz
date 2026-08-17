import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  invalidateManagedAgentQueriesInBackground,
  managedAgentsQueryKey,
} from "@/features/agents/hooks";
import { redeployManagedAgent } from "@/shared/api/tauriManagedAgents";
import type { ManagedAgent } from "@/shared/api/types";

export function useRedeployManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (pubkey: string) => redeployManagedAgent(pubkey),
    onSuccess: (updated) => {
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          if (!current) return current;
          return current.map((agent) =>
            agent.pubkey === updated.pubkey ? updated : agent,
          );
        },
      );
    },
    onSettled: () => {
      invalidateManagedAgentQueriesInBackground(queryClient);
    },
  });
}
