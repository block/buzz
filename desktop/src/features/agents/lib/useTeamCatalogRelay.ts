import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  fetchTeamCatalogPublications,
  type TeamCatalogPublication,
} from "@/features/agents/lib/teamCatalogRelay";
import { personasQueryKey, teamsQueryKey } from "@/features/agents/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { addTeamFromCatalog, setTeamShared } from "@/shared/api/tauriTeams";
import type {
  AgentTeam,
  TeamCatalogSourceCoordinate,
} from "@/shared/api/types";
import { KIND_TEAM_CATALOG } from "@/shared/constants/kinds";

/**
 * Team catalog reads and writes, keyed by community.
 *
 * Structurally the persona equivalent (`usePersonaCatalogRelay`) with the
 * kind and command swapped. Adding is the one genuine difference: it writes
 * personas as well as teams, so it invalidates both stores.
 */

export function teamCatalogQueryKey(communityId: string | null) {
  return ["team-catalog", communityId] as const;
}

export function useTeamCatalogQuery(communityId: string | null) {
  return useQuery<TeamCatalogPublication[]>({
    enabled: communityId !== null,
    queryKey: teamCatalogQueryKey(communityId),
    queryFn: fetchTeamCatalogPublications,
    staleTime: 30_000,
    refetchInterval: 120_000,
  });
}

export function useTeamCatalogLiveUpdates(communityId: string | null): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    if (!communityId) return;
    let disposed = false;
    let dispose: (() => Promise<void>) | null = null;

    const invalidate = () => {
      void queryClient.invalidateQueries({
        queryKey: teamCatalogQueryKey(communityId),
      });
    };

    void relayClient
      .subscribeLive({ kinds: [KIND_TEAM_CATALOG], limit: 0 }, invalidate)
      .then((unsubscribe) => {
        if (disposed) {
          void unsubscribe();
        } else {
          dispose = unsubscribe;
        }
      })
      .catch((error) => {
        console.error(
          "Couldn’t subscribe to the community team catalog",
          error,
        );
      });

    const unsubscribeReconnect = relayClient.subscribeToReconnects(invalidate);

    return () => {
      disposed = true;
      unsubscribeReconnect();
      if (dispose) void dispose();
    };
  }, [communityId, queryClient]);
}

export function useSetTeamCatalogSharedMutation(communityId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, shared }: { id: string; shared: boolean }) =>
      setTeamShared(id, shared),
    onSuccess: (result) => {
      queryClient.setQueryData<AgentTeam[]>(
        teamsQueryKey,
        (current) =>
          current?.map((team) =>
            team.id === result.team.id ? result.team : team,
          ) ?? [result.team],
      );
      void queryClient.invalidateQueries({
        queryKey: teamCatalogQueryKey(communityId),
      });
    },
  });
}

/**
 * Add a published team, then refresh both stores.
 *
 * The command copies every member as a local persona, so leaving the persona
 * query stale would show the new team with members the agents list does not
 * know about yet.
 */
export function useAddTeamFromCatalogMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (source: TeamCatalogSourceCoordinate & { eventId: string }) =>
      addTeamFromCatalog(source),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: teamsQueryKey }),
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
      ]);
    },
  });
}
