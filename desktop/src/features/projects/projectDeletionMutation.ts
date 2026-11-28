import type { QueryClient } from "@tanstack/react-query";

import type { Project, Repository } from "./projectModels";
import { deleteProject, deleteRepository } from "./projectDeletion";

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

/** Keep the shared project cache authoritative after deleting a repository announcement. */
export function repositoryDeletionMutationOptions(
  queryClient: QueryClient,
  deleteRepositoryFn: (
    repository: Repository,
  ) => Promise<void> = deleteRepository,
) {
  type DeletionResult = Awaited<ReturnType<typeof deleteRepositoryFn>>;
  return {
    mutationFn: deleteRepositoryFn,
    onSuccess: (_data: DeletionResult, repository: Repository) => {
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current
          .map((project) => ({
            ...project,
            repositories: project.repositories.filter(
              (item) => item.id !== repository.id,
            ),
            repositoryAddresses: project.repositoryAddresses.filter(
              (address) => address !== repository.repoAddress,
            ),
          }))
          .filter(
            (project) => !project.legacy || project.repositories.length > 0,
          ),
      );
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: projectsQueryKey }),
  };
}
