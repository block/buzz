import * as React from "react";

import type { ProjectPullRequest, Repository } from "@/features/projects/hooks";
import {
  getProjectLocalRepoFileContent,
  getProjectRepoFileContent,
} from "@/shared/api/projectGit";
import type { RepositoryFileContentSource } from "./useRepositoryFileContent";

type RepositoryFileContentSourceInput = {
  activeBranch: string | null;
  activeTag: { commit: string; name: string } | null;
  pullRequest: ProjectPullRequest | null;
  repository: Repository | null;
  reposDir?: string;
  selectedTag: string | null;
  source: "local" | "remote";
};

export function useRepositoryFileContentSource({
  activeBranch,
  activeTag,
  pullRequest,
  repository,
  reposDir,
  selectedTag,
  source,
}: RepositoryFileContentSourceInput) {
  return React.useMemo<RepositoryFileContentSource | undefined>(() => {
    if (!repository) return undefined;
    const effectiveSource = selectedTag ? "remote" : source;

    if (effectiveSource === "local") {
      return {
        cacheKey: [
          repository.id,
          "local",
          reposDir ?? "default",
          activeBranch ?? "default",
        ],
        load: (path) =>
          getProjectLocalRepoFileContent({
            reposDir,
            projectDtag: repository.dtag,
            cloneUrl: repository.cloneUrls[0] ?? null,
            path,
          }),
      };
    }

    const contentPullRequest = selectedTag ? null : pullRequest;
    const cloneUrl =
      contentPullRequest?.cloneUrls[0] ?? repository.cloneUrls[0];
    if (!cloneUrl) return undefined;
    const targetRef = activeTag
      ? `refs/tags/${activeTag.name}`
      : contentPullRequest
        ? `refs/nostr/${contentPullRequest.id}`
        : null;
    const targetCommit =
      activeTag?.commit ?? contentPullRequest?.commit ?? null;
    return {
      cacheKey: [
        repository.id,
        "remote",
        activeBranch ?? "default",
        targetRef ?? "default",
        targetCommit ?? "default",
      ],
      load: (path) =>
        getProjectRepoFileContent({
          cloneUrl,
          defaultBranch: activeBranch ?? repository.defaultBranch,
          targetRef,
          targetCommit,
          path,
        }),
    };
  }, [
    activeBranch,
    activeTag,
    pullRequest,
    repository,
    reposDir,
    selectedTag,
    source,
  ]);
}
