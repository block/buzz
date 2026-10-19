import { useQuery } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { fetchChannelTasks } from "./channelTaskFetch";
import type { Project } from "./projectModels";

/** Keep task reads separate across communities, identities, channels and repository sets. */
export function useChannelNextUp(
  channelId: string,
  projects: readonly Project[],
  pubkey: string,
) {
  const { activeCommunity } = useCommunities();
  const addresses = [
    ...new Set(
      projects.flatMap((project) =>
        project.repositories.map((repo) => repo.repoAddress),
      ),
    ),
  ].sort();
  return useQuery({
    queryKey: [
      "projects",
      "work-items",
      "channel-next-up",
      activeCommunity?.relayUrl,
      pubkey,
      channelId,
      addresses,
    ],
    enabled: addresses.length > 0 && Boolean(pubkey),
    queryFn: ({ signal }) => fetchChannelTasks(channelId, addresses, signal),
    staleTime: 10_000,
    retry: 1,
  });
}
