/**
 * REG-15: react-query hooks over the channel-task commands.
 *
 * Tasks are request/response by system property (the relay does no fanout on
 * task mutation and tasks are not nostr events — fire-#28 Q2), so these rely
 * on react-query's refetch-on-window-focus for freshness instead of a
 * subscription that cannot exist at any layer.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import {
  type ChannelTask,
  type TaskStatusFilter,
  createChannelTask,
  listChannelTasks,
  listMyWorkspaceTasks,
  setChannelTaskStatus,
} from "./channelTasks";

export const channelTasksKey = (channelId: string | null) => [
  "channel-tasks",
  channelId,
] as const;

const myWorkspaceTasksKey = ["my-workspace-tasks"] as const;

/**
 * Task list for the active workspace community. `channelId === null` means
 * the community's whole visible task set (the relay post-filters to channels
 * the caller can see) — that is the v1 screen's mode; a concrete channel id
 * scopes the same query for a channel-pane embed.
 */
export function useChannelTasks(channelId: string | null) {
  return useQuery({
    queryKey: channelTasksKey(channelId),
    queryFn: () => listChannelTasks({ channelId }),
  });
}

/** All visible tasks in the active community (channel filter unset). */
export function useCommunityTasks(status?: TaskStatusFilter) {
  return useQuery({
    queryKey: [...channelTasksKey(null), status ?? "all"] as const,
    queryFn: () => listChannelTasks({ status }),
  });
}

/**
 * Consolidated My-Tasks across the session's communities. The fan-in bound
 * (10 by recency) and the per-source INLINE error policy live in the Rust
 * command; the hook only supplies the relay bases.
 */
export function useMyWorkspaceTasks() {
  const { communities } = useCommunities();
  const relayBases = React.useMemo(
    () =>
      communities
        .slice(0, 10)
        .map((community) => community.relayUrl)
        .filter((url): url is string => typeof url === "string" && url !== ""),
    [communities],
  );
  return useQuery({
    queryKey: myWorkspaceTasksKey,
    queryFn: () => listMyWorkspaceTasks(relayBases),
    enabled: relayBases.length > 0,
  });
}

export function useCreateChannelTask(channelId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (title: string) => createChannelTask({ title, channelId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["channel-tasks"] });
      void queryClient.invalidateQueries({ queryKey: myWorkspaceTasksKey });
    },
  });
}

export function useSetChannelTaskStatus() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { taskId: string; status: ChannelTask["status"] }) =>
      setChannelTaskStatus(input.taskId, input.status as "open" | "done"),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["channel-tasks"] });
      void queryClient.invalidateQueries({ queryKey: myWorkspaceTasksKey });
    },
  });
}
