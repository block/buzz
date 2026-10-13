import { isGitHubCloneUrl } from "@/features/projects/lib/projectGitError";
import type { Repository } from "@/features/projects/projectModels";
import type { ProjectPullRequest } from "@/features/projects/projectPullRequests.mjs";
import type { ProjectRepoSnapshot } from "@/shared/api/types";

/**
 * True when the remote snapshot query can run for the repository. GitHub
 * waits for its repository state; Buzz and other git hosts load through git.
 */
export function githubRemoteSnapshotEnabled(input: {
  cloneUrl?: string | null;
  githubStateReady: boolean;
}) {
  if (!input.cloneUrl) return false;
  return !isGitHubCloneUrl(input.cloneUrl) || input.githubStateReady;
}

/** Route project-detail remote snapshots by the project clone URL host. */
export async function fetchProjectRepoSnapshotWith(
  project: Repository,
  branchName: string | null | undefined,
  _pullRequest: ProjectPullRequest | null | undefined,
  _tag: { name: string; commit: string } | null | undefined,
  loaders: {
    loadGithub: (input: {
      cloneUrl: string;
      ref: string;
    }) => Promise<ProjectRepoSnapshot>;
    loadBuzz: () => Promise<ProjectRepoSnapshot | null>;
  },
): Promise<ProjectRepoSnapshot | null> {
  const projectCloneUrl = project.cloneUrls[0];
  if (!projectCloneUrl) return null;
  if (isGitHubCloneUrl(projectCloneUrl)) {
    const ref = branchName ?? project.defaultBranch;
    if (!ref) return null;
    return loaders.loadGithub({ cloneUrl: projectCloneUrl, ref });
  }
  return loaders.loadBuzz();
}
