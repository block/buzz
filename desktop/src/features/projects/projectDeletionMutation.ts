import type { QueryClient } from "@tanstack/react-query";

import type { Project } from "./projectModels";
import { deleteProject } from "./projectDeletion";

export const projectsQueryKey = ["projects"] as const;

/** Keep the shared project cache authoritative even when publish ACK is lost. */
export function projectDeletionMutationOptions(
  queryClient: QueryClient,
  deleteProjectFn: (project: Project) => Promise<void> = deleteProject,
) {
  type DeletionResult = Awaited<ReturnType<typeof deleteProjectFn>>;
  return {
    mutationFn: deleteProjectFn,
    onSuccess: (_data: DeletionResult, project: Project) => {
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.filter((item) => item.id !== project.id),
      );
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: projectsQueryKey }),
  };
}
