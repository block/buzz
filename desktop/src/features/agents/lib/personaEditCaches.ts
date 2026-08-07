import type { QueryClient } from "@tanstack/react-query";

import { evictUsersBatchEntries } from "@/features/profile/hooks";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Refresh every cache a saved persona edit can invalidate.
 *
 * Shared by the plain edit mutation and the publish-on-save edit mutation so
 * the two cannot drift on what a saved edit refreshes.
 */
export async function invalidatePersonaEditCaches(
  queryClient: QueryClient,
  personaId: string,
): Promise<void> {
  // Evict per-pubkey users-batch-entry caches for agents linked to this
  // persona so the batch invalidation below refetches fresh profiles instead
  // of re-reading stale entries (mirrors useUpdateManagedAgentMutation).
  const agents = queryClient.getQueryData<ManagedAgent[]>(["managed-agents"]);
  if (agents) {
    evictUsersBatchEntries(
      queryClient,
      agents
        .filter((agent) => agent.personaId === personaId)
        .map((agent) => agent.pubkey.toLowerCase()),
    );
  }

  await Promise.all([
    queryClient.invalidateQueries({ queryKey: ["personas"] }),
    queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
    // Persona avatar changes re-sync linked agents' relay profiles;
    // invalidate cached user-profile and users-batch queries so the UI picks
    // up the updated kind:0 picture without waiting for staleTime expiry —
    // covers agent cards, message timelines, and member lists.
    queryClient.invalidateQueries({
      predicate: (query) =>
        query.queryKey[0] === "user-profile" ||
        query.queryKey[0] === "users-batch",
    }),
  ]);
}
