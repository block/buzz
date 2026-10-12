import {
  type QueryClient,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import { updateCachedChannelMemberDisplayName } from "@/features/channels/channelMemberProfileCache";
import { useOptionalCommunities } from "@/features/communities/useCommunities";
import { invokeTauri } from "@/shared/api/tauri";

import { fetchCommunityBots, uninstallCommunityBot } from "./lib/catalog";
import {
  completeCommunityBotInstall,
  completeCommunityBotRename,
} from "./lib/installFlow";
import {
  defaultRemoteAgentName,
  type CommunityBot,
  type CommunityBotsStatus,
  type RemoteOpenClawAgent,
  type ResolvedBotIdentity,
} from "./lib/types";

export const communityBotsQueryKey = ["communityBots"] as const;
export const communityBotsStatusQueryKey = ["communityBotsStatus"] as const;
export const communityBotsRemoteAgentsQueryKey = [
  "communityBotsRemoteAgents",
] as const;

function useCommunityBotsRelayUrl(): string {
  // Optional: rendered outside the provider (isolated dialogs/tests) it just has no relay instead of crashing the tree.
  return useOptionalCommunities()?.activeCommunity?.relayUrl ?? "";
}

export function useCommunityBotsQuery(enabled = true) {
  const relayUrl = useCommunityBotsRelayUrl();
  return useQuery({
    enabled,
    queryKey: [...communityBotsQueryKey, relayUrl],
    queryFn: () => fetchCommunityBots(relayUrl),
    staleTime: 15_000,
  });
}

export function useCommunityBotsStatusQuery(enabled = true) {
  return useQuery({
    enabled,
    queryKey: communityBotsStatusQueryKey,
    queryFn: () =>
      invokeTauri<CommunityBotsStatus>("community_bots_get_status"),
    staleTime: 5_000,
  });
}

export function useCommunityBotsRemoteAgentsQuery(enabled = false) {
  return useQuery({
    enabled,
    queryKey: communityBotsRemoteAgentsQueryKey,
    queryFn: () =>
      invokeTauri<RemoteOpenClawAgent[]>("community_bots_list_remote_agents"),
    staleTime: 15_000,
  });
}

export function useConnectCommunityBotsMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { url: string; password: string; token?: string }) =>
      invokeTauri<CommunityBotsStatus>("community_bots_connect", input),
    onSuccess: async (status) => {
      queryClient.setQueryData(communityBotsStatusQueryKey, status);
      if (status.state === "connected") {
        await queryClient.invalidateQueries({
          queryKey: communityBotsRemoteAgentsQueryKey,
        });
      }
    },
  });
}

export function useDisconnectCommunityBotsMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () =>
      invokeTauri<CommunityBotsStatus>("community_bots_disconnect"),
    onSuccess: async (status) => {
      queryClient.setQueryData(communityBotsStatusQueryKey, status);
      await queryClient.invalidateQueries({
        queryKey: communityBotsRemoteAgentsQueryKey,
      });
    },
  });
}

async function refreshCommunityBotAppearance(
  queryClient: QueryClient,
  pubkey: string,
  displayName: string,
): Promise<void> {
  await updateCachedChannelMemberDisplayName(queryClient, pubkey, displayName);
  await queryClient.invalidateQueries({ queryKey: ["relayMembers"] });
  await queryClient.invalidateQueries({ queryKey: ["users-batch"] });
  await queryClient.invalidateQueries({ queryKey: ["users-batch-entry"] });
  await queryClient.invalidateQueries({
    queryKey: ["user-profile", pubkey.toLowerCase()],
  });
}

export function useInstallCommunityBotMutation() {
  const queryClient = useQueryClient();
  const relayUrl = useCommunityBotsRelayUrl();
  return useMutation({
    mutationFn: async (input: {
      agent: RemoteOpenClawAgent;
      name?: string;
      pubkey?: string | null;
    }) => {
      const { agent } = input;
      const identity = await invokeTauri<ResolvedBotIdentity>(
        "community_bots_resolve_identity",
        {
          agentId: agent.id,
          pubkey: input.pubkey ?? agent.pubkey ?? null,
        },
      );
      if (identity.minted) {
        throw new Error(
          "Install refused to mint a local nsec. Use the OpenClaw VPS Buzz public key.",
        );
      }
      const installed = queryClient.getQueryData<CommunityBot[]>([
        ...communityBotsQueryKey,
        relayUrl,
      ]);
      const current = installed ?? (await fetchCommunityBots(relayUrl));
      return completeCommunityBotInstall({
        current,
        agent,
        displayName: input.name ?? defaultRemoteAgentName(agent),
        identity,
        relayUrl,
      });
    },
    onSuccess: async (next, input) => {
      queryClient.setQueryData([...communityBotsQueryKey, relayUrl], next);
      const bot = next.find((entry) => entry.id === input.agent.id);
      if (bot) {
        await refreshCommunityBotAppearance(queryClient, bot.pubkey, bot.name);
      }
    },
  });
}

export function useRenameCommunityBotMutation() {
  const queryClient = useQueryClient();
  const relayUrl = useCommunityBotsRelayUrl();
  return useMutation({
    mutationFn: async (input: { bot: CommunityBot; name: string }) => {
      const installed = queryClient.getQueryData<CommunityBot[]>([
        ...communityBotsQueryKey,
        relayUrl,
      ]);
      const current = installed ?? (await fetchCommunityBots(relayUrl));
      return completeCommunityBotRename({
        current,
        bot: input.bot,
        displayName: input.name,
        relayUrl,
      });
    },
    onSuccess: async (next, input) => {
      queryClient.setQueryData([...communityBotsQueryKey, relayUrl], next);
      const bot = next.find((entry) => entry.id === input.bot.id);
      if (bot) {
        await refreshCommunityBotAppearance(queryClient, bot.pubkey, bot.name);
      }
    },
  });
}

export function useUninstallCommunityBotMutation() {
  const queryClient = useQueryClient();
  const relayUrl = useCommunityBotsRelayUrl();
  return useMutation({
    mutationFn: async (bot: CommunityBot) => {
      const installed = queryClient.getQueryData<CommunityBot[]>([
        ...communityBotsQueryKey,
        relayUrl,
      ]);
      const current = installed ?? (await fetchCommunityBots(relayUrl));
      return uninstallCommunityBot(current, bot, relayUrl);
    },
    onSuccess: async (next) => {
      queryClient.setQueryData([...communityBotsQueryKey, relayUrl], next);
      await queryClient.invalidateQueries({ queryKey: ["relayMembers"] });
    },
  });
}
