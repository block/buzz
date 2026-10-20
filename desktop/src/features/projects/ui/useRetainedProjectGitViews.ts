import * as React from "react";

import {
  type ProjectPullRequest,
  type ProjectRepoDiff,
  type ProjectRepoSnapshot,
  type Repository,
  useProjectLocalRepoDiffQuery,
  useProjectLocalRepoSnapshotQuery,
  useProjectRepoDiffQuery,
  useProjectRepoSnapshotQuery,
} from "@/features/projects/hooks";
import {
  currentPullRequestForSelection,
  retainLatestByKey,
  reviewDiffWorkspaceBranch,
  shouldReplaceRetainedPullRequest,
} from "@/features/projects/lib/projectReviewDisplay";
import { normalizeRepositoryUrl } from "@/features/projects/lib/projectsViewHelpers";
import { useProjectCommitDiffQuery } from "@/features/projects/useProjectCommitDiff";
import { snapshotHasContent } from "./projectDetailHelpers";

/** Keep the active review identity stable while the pull-request list refetches. */
export function useRetainedPullRequestSelection({
  activeBranch,
  isFetching,
  pullRequests,
  repository,
  selectedPullRequestId,
}: {
  activeBranch: string | null | undefined;
  isFetching: boolean;
  pullRequests: ProjectPullRequest[] | undefined;
  repository: Repository | null | undefined;
  selectedPullRequestId: string | null;
}) {
  const currentBranchPullRequest = React.useMemo(() => {
    const projectRepositories = new Set(
      (repository?.cloneUrls ?? []).map(normalizeRepositoryUrl),
    );
    const matches =
      pullRequests?.filter(
        (pullRequest) =>
          pullRequest.branchName === activeBranch &&
          pullRequest.cloneUrls.some((cloneUrl) =>
            projectRepositories.has(normalizeRepositoryUrl(cloneUrl)),
          ),
      ) ?? [];
    return matches.length === 1 ? matches[0] : null;
  }, [activeBranch, pullRequests, repository?.cloneUrls]);
  const selectedBranchPullRequestKey = `${repository?.id ?? "none"}:${activeBranch}`;
  const selectedBranchPullRequestCache = React.useRef({
    key: selectedBranchPullRequestKey,
    value: currentBranchPullRequest,
  });
  const selectedBranchPullRequest = retainLatestByKey(
    selectedBranchPullRequestCache,
    selectedBranchPullRequestKey,
    currentBranchPullRequest,
    (next) => shouldReplaceRetainedPullRequest(next, isFetching),
  );
  const openBranchPullRequest =
    selectedBranchPullRequest?.status === "Open" ||
    selectedBranchPullRequest?.status === "Draft"
      ? selectedBranchPullRequest
      : null;
  const currentSelectedPullRequest = currentPullRequestForSelection({
    pullRequests,
    selectedPullRequestId,
  });
  const selectedPullRequestKey = [
    repository?.id ?? "none",
    selectedPullRequestId ?? "none",
  ].join(":");
  const selectedPullRequestCache = React.useRef({
    key: selectedPullRequestKey,
    value: currentSelectedPullRequest,
  });
  const selectedPullRequest = retainLatestByKey(
    selectedPullRequestCache,
    selectedPullRequestKey,
    currentSelectedPullRequest,
    (next) => shouldReplaceRetainedPullRequest(next, isFetching),
  );
  const activeRepoPullRequest = selectedPullRequestId
    ? selectedPullRequest
    : selectedBranchPullRequest;
  return {
    activeRepoPullRequest,
    openBranchPullRequest,
    selectedBranchPullRequest,
    selectedPullRequest,
  };
}

/** Keep a populated repository snapshot on screen across transient refetches. */
export function useRetainedRepoSnapshot(
  cacheKey: string,
  data: ProjectRepoSnapshot | null | undefined,
  isFetching: boolean,
) {
  const cache = React.useRef({
    key: cacheKey,
    value: data,
  });
  return retainLatestByKey(
    cache,
    cacheKey,
    data,
    (next, previous) =>
      snapshotHasContent(next) || !snapshotHasContent(previous) || !isFetching,
  );
}

/** Keep a populated review diff on screen across empty refetch results. */
export function useRetainedRepoDiff(
  cacheKey: string,
  data: ProjectRepoDiff | null | undefined,
  isFetching: boolean,
) {
  const cache = React.useRef({
    key: cacheKey,
    value: data,
  });
  return retainLatestByKey(cache, cacheKey, data, (next, previous) => {
    const nextFiles = next?.files.length ?? 0;
    const previousFiles = previous?.files.length ?? 0;
    if (previousFiles > 0 && nextFiles === 0) return !isFetching;
    return next !== undefined || !isFetching;
  });
}

/** Load and retain repository snapshot/diff queries for the project detail view. */
export function useProjectDetailGitViews({
  activeBranch,
  activeRepoPullRequest,
  activeTag,
  isBuzzHost,
  repository,
  reposDir,
  repoSource,
  selectedBranchPullRequest,
  selectedCommitHash,
  selectedTag,
}: {
  activeBranch: string | null | undefined;
  activeRepoPullRequest: ProjectPullRequest | null | undefined;
  activeTag: { commit: string; name: string } | null;
  isBuzzHost: boolean;
  repository: Repository | null | undefined;
  reposDir?: string | null;
  repoSource: "local" | "remote";
  selectedBranchPullRequest: ProjectPullRequest | null;
  selectedCommitHash: string | null;
  selectedTag: string | null;
}) {
  const repoSnapshotQuery = useProjectRepoSnapshotQuery(
    repository,
    activeBranch,
    selectedTag ? null : selectedBranchPullRequest,
    activeTag,
    isBuzzHost,
  );
  const displayedRepoSnapshot = useRetainedRepoSnapshot(
    `${repository?.id}:${activeBranch}:${selectedTag ? activeTag?.name : selectedBranchPullRequest?.id}:${selectedTag ? activeTag?.commit : selectedBranchPullRequest?.commit}`,
    repoSnapshotQuery.data,
    repoSnapshotQuery.isFetching,
  );
  const reviewDiffBranch = reviewDiffWorkspaceBranch({
    activeBranch,
    defaultBranch: repository?.defaultBranch,
    pullRequest: activeRepoPullRequest,
  });
  const repoDiffQuery = useProjectRepoDiffQuery(
    repository,
    reviewDiffBranch,
    activeRepoPullRequest,
    repoSource === "remote",
  );
  const localRepoDiffQuery = useProjectLocalRepoDiffQuery(
    repository,
    reposDir,
    reviewDiffBranch,
    activeRepoPullRequest,
    repoSource === "local" && Boolean(activeRepoPullRequest),
  );
  const commitDiffQuery = useProjectCommitDiffQuery(
    repository,
    selectedCommitHash,
    repoSource,
    reposDir,
  );
  const localRepoSnapshotQuery = useProjectLocalRepoSnapshotQuery(
    repository,
    reposDir,
    activeBranch,
  );
  const displayedRepoDiff = useRetainedRepoDiff(
    `${repository?.id ?? "none"}:${repoSource}:${reviewDiffBranch}:${activeRepoPullRequest?.id}:${activeRepoPullRequest?.commit}`,
    repoSource === "local" ? localRepoDiffQuery.data : repoDiffQuery.data,
    repoSource === "local"
      ? localRepoDiffQuery.isFetching
      : repoDiffQuery.isFetching,
  );
  return {
    commitDiffQuery,
    displayedRepoDiff,
    displayedRepoDiffError:
      repoSource === "local" ? localRepoDiffQuery.error : repoDiffQuery.error,
    displayedRepoDiffLoading: (repoSource === "local"
      ? localRepoDiffQuery
      : repoDiffQuery
    ).isLoading,
    displayedRepoSnapshot,
    localRepoSnapshotQuery,
    repoSnapshotQuery,
  };
}
