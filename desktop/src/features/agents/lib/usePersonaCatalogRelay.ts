import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  fetchPersonaCatalogPublications,
  publishPersonaToCatalog,
  unpublishPersonaFromCatalog,
  type PersonaCatalogPublication,
} from "@/features/agents/lib/personaCatalogRelay";
import { relayClient } from "@/shared/api/relayClient";
import { KIND_PERSONA_CATALOG } from "@/shared/constants/kinds";

export function personaCatalogQueryKey(communityId: string | null) {
  return ["persona-catalog", communityId] as const;
}

export function usePersonaCatalogQuery(communityId: string | null) {
  return useQuery<PersonaCatalogPublication[]>({
    enabled: communityId !== null,
    queryKey: personaCatalogQueryKey(communityId),
    queryFn: fetchPersonaCatalogPublications,
    staleTime: 30_000,
    refetchInterval: 120_000,
  });
}

export function usePersonaCatalogLiveUpdates(communityId: string | null): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    if (!communityId) return;
    let disposed = false;
    let dispose: (() => Promise<void>) | null = null;

    void relayClient
      .subscribeLive({ kinds: [KIND_PERSONA_CATALOG], limit: 0 }, () => {
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
          "Failed to subscribe to the community agent catalog",
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

export function usePublishPersonaCatalogMutation(communityId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: publishPersonaToCatalog,
    onSuccess: (publication) => {
      queryClient.setQueryData<PersonaCatalogPublication[]>(
        personaCatalogQueryKey(communityId),
        (current) => [
          publication,
          ...(current ?? []).filter(
            (candidate) =>
              candidate.ownerPubkey !== publication.ownerPubkey ||
              candidate.sourcePersonaId !== publication.sourcePersonaId,
          ),
        ],
      );
    },
  });
}

export function useUnpublishPersonaCatalogMutation(communityId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: unpublishPersonaFromCatalog,
    onSuccess: (publication) => {
      queryClient.setQueryData<PersonaCatalogPublication[]>(
        personaCatalogQueryKey(communityId),
        (current) => [
          publication,
          ...(current ?? []).filter(
            (candidate) =>
              candidate.ownerPubkey !== publication.ownerPubkey ||
              candidate.sourcePersonaId !== publication.sourcePersonaId,
          ),
        ],
      );
    },
  });
}
