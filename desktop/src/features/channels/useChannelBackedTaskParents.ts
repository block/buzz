import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  taskParentChannel,
  parseChannelBackedTask,
  type ChannelBackedTask,
} from "@/features/channels/lib/channelBackedTask";
import { getCanvases } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";

export const TaskChannelBranches = React.createContext(
  new Map<string, NonNullable<ChannelBackedTask["branch"]>>(),
);

export function useChannelBackedTaskParents(channels: Channel[]) {
  const queryClient = useQueryClient();
  React.useEffect(
    () =>
      queryClient.getQueryCache().subscribe((event) => {
        if (
          event.type === "updated" &&
          event.action.type === "success" &&
          event.query.queryKey[0] === "channel-canvas"
        ) {
          void queryClient.invalidateQueries({
            queryKey: ["channel-backed-task-canvases"],
          });
        }
      }),
    [queryClient],
  );
  const channelIds = React.useMemo(
    () => channels.map((channel) => channel.id).sort(),
    [channels],
  );
  const canvases = useQuery({
    queryKey: ["channel-backed-task-canvases", channelIds],
    queryFn: () => getCanvases(channelIds),
    enabled: channelIds.length > 0,
    staleTime: 60_000,
  });

  return React.useMemo(() => {
    const parents = new Map<string, string>();
    const branches = new Map<
      string,
      NonNullable<ChannelBackedTask["branch"]>
    >();
    for (const channelId of channelIds) {
      const parentId = taskParentChannel(canvases.data?.[channelId]?.content);
      if (parentId) parents.set(channelId, parentId);
      const branch = parseChannelBackedTask(
        canvases.data?.[channelId]?.content,
      )?.branch;
      if (branch) branches.set(channelId, branch);
    }
    return { parents, branches };
  }, [canvases.data, channelIds]);
}
