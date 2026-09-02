import { useMutation, useQueryClient } from "@tanstack/react-query";

import { projectsQueryKey } from "@/features/projects/hooks";
import type { Project } from "@/features/projects/projectModels";
import { changeProjectRelatedChannels } from "@/features/projects/projectState";

export type ChangeProjectRelatedChannelsInput = {
  add?: string[];
  project: Project;
  remove?: string[];
};

export function useChangeProjectRelatedChannelsMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      add = [],
      project,
      remove = [],
    }: ChangeProjectRelatedChannelsInput) =>
      changeProjectRelatedChannels(project, { add, remove }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: projectsQueryKey });
    },
  });
}
