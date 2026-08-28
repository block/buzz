/**
 * REG-16: the one new mutation — assign an owner to a channel task.
 *
 * Mirrors `useSetChannelTaskStatus` (useChannelTasks.ts:83-93) exactly,
 * including its `channel-tasks` + my-workspace invalidation, so freshness
 * behaves identically to the write REG-15 already shipped.
 *
 * Authz is inherited: the relay is the sole authority on whether the caller
 * may assign. We deliberately do NOT pre-filter candidates by a client-side
 * permission guess — a guess that disagrees with the relay would be a lie in
 * either direction (work/REG-16/reflecting.md §Authz).
 */
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { setChannelTaskAssignee } from "./channelTasks";

export function useSetChannelTaskAssignee() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { taskId: string; assignee: string | null }) =>
      setChannelTaskAssignee(input.taskId, input.assignee),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["channel-tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["my-workspace-tasks"] });
    },
  });
}
