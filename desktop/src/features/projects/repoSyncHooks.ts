import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getProjectRepoSyncStatus,
  pullProjectLocalRepository,
  pushProjectLocalRepository,
} from "@/shared/api/projectGit";
import type { Project } from "@/features/projects/hooks";

/** Local-vs-remote git sync status for a project checkout (ahead/behind
 * counts, push/pull availability). Polls gently — each check runs a
 * `git fetch` — and refetches on focus to catch the common "committed in
 * a terminal, switched back to the app" flow. */
export function useProjectRepoSyncStatusQuery(
  project: Project | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(project?.cloneUrls[0]),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-sync-status",
      reposDir ?? "default",
      selectedBranch ?? "default",
    ],
    queryFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return getProjectRepoSyncStatus({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        defaultBranch: selectedBranch,
      });
    },
    staleTime: 10_000,
    refetchInterval: 60_000,
    refetchOnWindowFocus: true,
    retry: 1,
  });
}

/** Pushes local commits to the project remote. */
export function usePushProjectLocalRepositoryMutation(
  project: Project | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
) {
  const queryClient = useQueryClient();
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useMutation({
    mutationFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return pushProjectLocalRepository({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        defaultBranch: selectedBranch,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none"],
      });
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

/** Fast-forwards the local checkout to the remote branch head. */
export function usePullProjectLocalRepositoryMutation(
  project: Project | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
) {
  const queryClient = useQueryClient();
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useMutation({
    mutationFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return pullProjectLocalRepository({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        defaultBranch: selectedBranch,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none"],
      });
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}
