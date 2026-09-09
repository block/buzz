import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  channelIdFromLink,
  parseChannelBackedTask,
} from "@/features/channels/lib/channelBackedTask";
import { getCanvases } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";

export function useChannelBackedTaskParents(channels: Channel[]) {
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
    for (const channelId of channelIds) {
      const task = parseChannelBackedTask(canvases.data?.[channelId]?.content);
      const parentId = task && channelIdFromLink(task.parentChannel);
      if (parentId) parents.set(channelId, parentId);
    }
    return parents;
  }, [canvases.data, channelIds]);
}
