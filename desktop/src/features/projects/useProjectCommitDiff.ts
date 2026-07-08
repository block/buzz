import { useQuery } from "@tanstack/react-query";

import {
  getProjectLocalRepoDiff,
  getProjectRepoDiff,
} from "@/shared/api/projectGit";
import type { ProjectRepoDiff } from "@/shared/api/types";
import type { Project } from "./hooks";

async function fetchProjectCommitDiff(
  project: Project,
  commitHash: string,
  repoSource: "remote" | "local",
  reposDir: string | null | undefined,
): Promise<ProjectRepoDiff> {
  if (repoSource === "local") {
    // Passing only the target commit (no base branch/commit) makes the
    // backend diff the commit against its parent.
    const local = await getProjectLocalRepoDiff({
      reposDir,
      projectDtag: project.dtag,
      cloneUrl: project.cloneUrls[0] ?? null,
      targetCommit: commitHash,
    });
    if (local) return local;
  }

  const cloneUrl = project.cloneUrls[0];
  if (!cloneUrl) {
    throw new Error("This project has no clone URL to load the commit from.");
  }
  return getProjectRepoDiff({
    cloneUrl,
    defaultBranch: project.defaultBranch,
    targetCommit: commitHash,
  });
}

/**
 * Diff of a single commit against its parent, for the commit detail view.
 * Prefers the local checkout when the repository source is "local" and falls
 * back to a remote fetch when no checkout exists.
 */
export function useProjectCommitDiffQuery(
  project: Project | null | undefined,
  commitHash: string | null,
  repoSource: "remote" | "local",
  reposDir?: string | null,
) {
  return useQuery({
    enabled: Boolean(project && commitHash),
    queryKey: [
      "project",
      project?.id ?? "none",
      "commit-diff",
      repoSource,
      commitHash ?? "none",
    ],
    queryFn: () => {
      if (!project || !commitHash) {
        return Promise.reject(new Error("No commit selected."));
      }
      return fetchProjectCommitDiff(project, commitHash, repoSource, reposDir);
    },
    // A commit's diff is immutable, so never refetch it while cached.
    staleTime: Number.POSITIVE_INFINITY,
    retry: 1,
  });
}
