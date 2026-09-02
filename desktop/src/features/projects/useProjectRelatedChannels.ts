import { useMutation, useQueryClient } from "@tanstack/react-query";

import { projectsQueryKey } from "@/features/projects/projectDeletionMutation";
import type { Project } from "@/features/projects/projectModels";
import { setProjectRelatedChannel } from "@/features/projects/projectRelatedChannelCommands";

export function useSetProjectRelatedChannelMutation(project: Project) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { channelId: string; linked: boolean }) =>
      setProjectRelatedChannel({
        ...input,
        projectAddress: project.projectAddress,
      }),
    onSuccess: (_result, input) => {
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.map((candidate) => {
          if (candidate.projectAddress !== project.projectAddress) {
            return candidate;
          }
          const relatedChannelIds = new Set(candidate.relatedChannelIds);
          if (input.linked) {
            relatedChannelIds.add(input.channelId);
          } else {
            relatedChannelIds.delete(input.channelId);
          }
          return { ...candidate, relatedChannelIds: [...relatedChannelIds] };
        }),
      );
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
    },
  });
}
