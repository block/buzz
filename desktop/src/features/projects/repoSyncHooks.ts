import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cloneProjectRepository,
  getProjectRepoSyncStatus,
  pullProjectLocalRepository,
  pushProjectLocalRepository,
} from "@/shared/api/projectGit";
import type { Project, ProjectPullRequest } from "@/features/projects/hooks";
import { publishProjectPullRequestUpdate } from "./pullRequestMutations";

/** Local-vs-remote git sync status for a project checkout (ahead/behind
 * counts, push/pull availability). Polls gently — each check runs a
 * `git fetch` — and refetches on focus to catch the common "committed in
 * a terminal, switched back to the app" flow. */
export function useProjectRepoSyncStatusQuery(
  project: Project | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
  baseBranch?: string | null,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;
  const selectedBaseBranch = baseBranch ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(project?.cloneUrls[0]),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-sync-status",
      reposDir ?? "default",
      selectedBranch ?? "default",
      selectedBaseBranch ?? "default",
    ],
    queryFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return getProjectRepoSyncStatus({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        branchName: selectedBranch,
        baseBranch: selectedBaseBranch,
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
  pullRequest?: ProjectPullRequest | null,
) {
  const queryClient = useQueryClient();
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useMutation({
    mutationFn: async () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      const result = await pushProjectLocalRepository({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        branchName: selectedBranch,
        baseBranch: project.defaultBranch,
      });
      let pullRequestUpdate:
        | { status: "skipped" | "unchanged" | "updated" }
        | { status: "failed"; error: string } = { status: "skipped" };
      if (
        pullRequest &&
        (pullRequest.status === "Open" || pullRequest.status === "Draft")
      ) {
        try {
          const updated = await publishProjectPullRequestUpdate({
            commit: result.commit,
            mergeBase: result.mergeBase,
            project,
            pullRequest,
          });
          pullRequestUpdate = {
            status: updated ? "updated" : "unchanged",
          };
        } catch (error) {
          pullRequestUpdate = {
            status: "failed",
            error:
              error instanceof Error
                ? error.message
                : "The pull request update could not be published.",
          };
        }
      }
      return { ...result, pullRequestUpdate };
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none"],
      });
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

/** Clones a project into the workspace repositories directory. */
export function useCloneProjectRepositoryMutation(
  project: Project | null | undefined,
  reposDir?: string | null,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return cloneProjectRepository({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        defaultBranch: project.defaultBranch,
      });
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["project", project?.id ?? "none", "local-repo-snapshot"],
        }),
        queryClient.invalidateQueries({
          queryKey: ["project", project?.id ?? "none", "repo-sync-status"],
        }),
        queryClient.invalidateQueries({
          queryKey: ["projects", "local-repositories"],
        }),
      ]);
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
        branchName: selectedBranch,
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
