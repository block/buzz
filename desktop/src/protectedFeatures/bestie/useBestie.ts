import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  managedAgentRuntimesQueryKey,
  useManagedAgentRuntimeAction,
  useManagedAgentRuntimesQuery,
} from "@/features/agents/managedAgentRuntimeHooks";
import {
  canonicalRelayUrl,
  findManagedAgentRuntime,
  managedAgentPairAction,
} from "@/features/agents/managedAgentRuntimeStatus";
import {
  channelsQueryKey,
  upsertCachedChannel,
} from "@/features/channels/hooks";
import { dmVisibilityQueryKeyFor } from "@/features/channels/useHiddenDmIds";
import { useCommunities } from "@/features/communities/useCommunities";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, ManagedAgent } from "@/shared/api/types";
import {
  assignBestie,
  clearBestieAssignment,
  getBestieAssignment,
  resolveBestieConversation,
  type BestieScope,
} from "./api";
import { findAssignedLocalAgent } from "./findAssignedLocalAgent";

export function bestieAssignmentQueryKey(
  relayUrl: string,
  ownerPubkey: string,
) {
  return [
    "bestie-assignment",
    canonicalRelayUrl(relayUrl) ?? relayUrl,
    ownerPubkey.toLowerCase(),
  ] as const;
}

export function useBestieAssignmentQuery(enabled = true) {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const ownerPubkey = identityQuery.data?.pubkey ?? "";
  const scope: BestieScope | null =
    relayUrl && ownerPubkey
      ? {
          expectedRelayUrl: relayUrl,
          expectedSignerPubkey: ownerPubkey,
        }
      : null;
  const queryKey = bestieAssignmentQueryKey(relayUrl, ownerPubkey);
  const assignmentQuery = useQuery({
    enabled: enabled && scope !== null,
    queryKey,
    queryFn: () => {
      if (!scope) return null;
      return getBestieAssignment(scope);
    },
  });

  return { assignmentQuery, ownerPubkey, queryKey, relayUrl, scope };
}

export function useBestie() {
  const queryClient = useQueryClient();
  const { goChannel } = useAppNavigation();
  const { assignmentQuery, ownerPubkey, queryKey, relayUrl, scope } =
    useBestieAssignmentQuery();
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: scope !== null });
  const runtimesQuery = useManagedAgentRuntimesQuery({
    enabled: scope !== null,
  });
  const runtimeAction = useManagedAgentRuntimeAction();
  const assignedAgent = findAssignedLocalAgent(
    managedAgentsQuery.data ?? [],
    assignmentQuery.data,
  );
  const presenceQuery = usePresenceQuery(
    assignedAgent ? [assignedAgent.pubkey] : [],
    { enabled: assignedAgent !== null },
  );
  const presenceStatus = assignedAgent
    ? (presenceQuery.data?.[assignedAgent.pubkey.toLowerCase()] ?? "offline")
    : undefined;
  const runtime = assignedAgent
    ? findManagedAgentRuntime(
        runtimesQuery.data ?? [],
        assignedAgent.pubkey,
        relayUrl,
      )
    : undefined;

  const assignMutation = useMutation({
    mutationFn: (agent: ManagedAgent) => {
      if (!scope) throw new Error("Bestie is unavailable outside a community");
      return assignBestie(agent.pubkey, scope);
    },
    onSuccess: (assignment) => {
      queryClient.setQueryData(queryKey, assignment);
    },
  });
  const clearMutation = useMutation({
    mutationFn: () => {
      if (!scope) throw new Error("Bestie is unavailable outside a community");
      return clearBestieAssignment(scope);
    },
    onSuccess: () => {
      queryClient.setQueryData(queryKey, null);
    },
  });
  const resolveMutation = useMutation({
    mutationFn: () => {
      if (!scope) throw new Error("Bestie is unavailable outside a community");
      return resolveBestieConversation(scope);
    },
    onSuccess: (channel) => {
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, channel),
      );
      queryClient.setQueryData<Set<string>>(
        dmVisibilityQueryKeyFor(relayUrl, ownerPubkey),
        (current) => {
          const next = new Set(current);
          next.delete(channel.id);
          return next;
        },
      );
    },
  });

  const ensureAgentRunning = async () => {
    if (assignedAgent) {
      const action = managedAgentPairAction(runtime);
      if (action !== "stop") {
        await runtimeAction.mutateAsync({
          action,
          pubkey: assignedAgent.pubkey,
          relayUrl,
        });
        await queryClient.invalidateQueries({
          queryKey: managedAgentRuntimesQueryKey,
        });
      }
    }
  };

  const resolveConversation = async () => {
    return resolveMutation.mutateAsync();
  };

  const openConversation = async (draft?: string) => {
    await ensureAgentRunning();
    const channel = await resolveConversation();
    await goChannel(channel.id, draft ? { autoSend: draft } : undefined);
  };

  return {
    assignedAgent,
    assignment: assignmentQuery.data ?? null,
    assignmentError: assignmentQuery.error,
    assignAgent: assignMutation.mutateAsync,
    clearAssignment: clearMutation.mutateAsync,
    ensureAgentRunning,
    isAssigning: assignMutation.isPending,
    isLoading:
      assignmentQuery.isLoading ||
      managedAgentsQuery.isLoading ||
      runtimesQuery.isLoading,
    isOpening: resolveMutation.isPending || runtimeAction.isPending,
    openConversation,
    presenceStatus,
    resolveConversation,
    runtime,
  };
}
