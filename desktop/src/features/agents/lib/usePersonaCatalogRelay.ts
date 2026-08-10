import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  fetchPersonaCatalogPublications,
  type PersonaCatalogPublication,
} from "@/features/agents/lib/personaCatalogRelay";
import { invalidatePersonaEditCaches } from "@/features/agents/lib/personaEditCaches";
import { relayClient } from "@/shared/api/relayClient";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";
import {
  setPersonaShared,
  updatePersonaAndPublish,
} from "@/shared/api/tauriPersonas";
import type { AgentPersona, UpdatePersonaInput } from "@/shared/api/types";
import { KIND_PERSONA } from "@/shared/constants/kinds";

export function personaCatalogQueryKey(communityId: string | null) {
  return ["persona-catalog", communityId] as const;
}

export function usePersonaCatalogQuery(communityId: string | null) {
  const refetchInterval = useFocusedRefetchInterval(120_000);
  return useQuery<PersonaCatalogPublication[]>({
    enabled: communityId !== null,
    queryKey: personaCatalogQueryKey(communityId),
    queryFn: fetchPersonaCatalogPublications,
    staleTime: 30_000,
    refetchInterval,
    refetchOnWindowFocus: true,
  });
}

export function usePersonaCatalogLiveUpdates(communityId: string | null): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    if (!communityId) return;
    let disposed = false;
    let dispose: (() => Promise<void>) | null = null;

    void relayClient
      .subscribeLive({ kinds: [KIND_PERSONA], limit: 0 }, () => {
        void queryClient.invalidateQueries({
          queryKey: personaCatalogQueryKey(communityId),
        });
      })
      .then((unsubscribe) => {
        if (disposed) {
          void unsubscribe();
        } else {
          dispose = unsubscribe;
        }
      })
      .catch((error) => {
        console.error(
          "Couldn’t subscribe to the community agent catalog",
          error,
        );
      });

    const unsubscribeReconnect = relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({
        queryKey: personaCatalogQueryKey(communityId),
      });
    });

    return () => {
      disposed = true;
      unsubscribeReconnect();
      if (dispose) void dispose();
    };
  }, [communityId, queryClient]);
}

export function useSetPersonaCatalogSharedMutation(communityId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, shared }: { id: string; shared: boolean }) =>
      setPersonaShared(id, shared),
    onSuccess: (result) => {
      queryClient.setQueryData<AgentPersona[]>(
        ["personas"],
        (current) =>
          current?.map((persona) =>
            persona.id === result.persona.id ? result.persona : persona,
          ) ?? [result.persona],
      );
      void queryClient.invalidateQueries({
        queryKey: personaCatalogQueryKey(communityId),
      });
    },
  });
}

/**
 * Save a persona edit and publish its catalog head, reporting the relay's
 * verdict.
 *
 * The plain edit mutation only enqueues the head best-effort, so it cannot back
 * the "Save and publish" promise. This awaits the relay and additionally
 * refreshes the catalog query, since the published edit changes what the
 * catalog shows.
 */
export function useUpdatePersonaAndPublishMutation(communityId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: UpdatePersonaInput) => updatePersonaAndPublish(input),
    onSettled: async (_data, _error, variables) => {
      await Promise.all([
        invalidatePersonaEditCaches(queryClient, variables.id),
        queryClient.invalidateQueries({
          queryKey: personaCatalogQueryKey(communityId),
        }),
      ]);
    },
  });
}
