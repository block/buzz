type RetainLatestByKeyCache<T> = {
  current: {
    key: string;
    value: T;
  };
};

/**
 * Keep the latest accepted value for a stable key across transient empties.
 * A new key always takes `value` immediately (real navigation).
 */
export function retainLatestByKey<T>(
  cache: RetainLatestByKeyCache<T>,
  key: string,
  value: T,
  shouldReplace: (next: T, previous: T) => boolean,
): T {
  if (cache.current.key !== key) {
    cache.current = { key, value };
    return value;
  }
  if (shouldReplace(value, cache.current.value)) {
    cache.current.value = value;
  }
  return cache.current.value;
}

/**
 * Keep a selected review through transient empty refetches. Once the fetch
 * is idle, accept the completed result — including null — so the rendered
 * identity can match the diff-query identity.
 */
export function shouldReplaceRetainedPullRequest(
  next: unknown,
  isFetching: boolean,
): boolean {
  return Boolean(next) || !isFetching;
}

/**
 * Resolve the current review for a selection. An explicit ID that is missing
 * from the list is `null` so a completed refetch can clear it; pass
 * `fallback` only when no ID is selected (branch auto-select).
 */
export function currentPullRequestForSelection<T extends { id: string }>({
  fallback = null,
  pullRequests,
  selectedPullRequestId,
}: {
  fallback?: T | null;
  pullRequests: readonly T[] | undefined;
  selectedPullRequestId: string | null;
}): T | null {
  if (selectedPullRequestId) {
    return (
      pullRequests?.find((item) => item.id === selectedPullRequestId) ?? null
    );
  }
  return fallback;
}

/**
 * Which body to render under a review's Files changed section.
 * A populated diff must keep the files panel mounted even when the repository
 * snapshot briefly looks unavailable — swapping in the unavailable placeholder
 * is the files-section flicker.
 */
export function projectReviewFilesChangedBody({
  hasPopulatedDiff,
  hasSelectedPullRequest,
  repositoryUnavailable,
}: {
  hasPopulatedDiff: boolean;
  hasSelectedPullRequest: boolean;
  repositoryUnavailable: boolean;
}): "files" | "unavailable" | null {
  if (hasSelectedPullRequest && (hasPopulatedDiff || !repositoryUnavailable)) {
    return "files";
  }
  if (repositoryUnavailable) return "unavailable";
  return null;
}

/**
 * Workspace branch used to fetch a review diff.
 * A selected review is `target...head`; do not key that query on the head
 * branch or the workspace picker, or Files changed will swap with the
 * default-branch snapshot.
 */
export function reviewDiffWorkspaceBranch({
  activeBranch,
  defaultBranch,
  pullRequest,
}: {
  activeBranch: string | null | undefined;
  defaultBranch: string | null | undefined;
  pullRequest: { targetBranch: string | null } | null | undefined;
}): string | null | undefined {
  if (!pullRequest) return activeBranch;
  return pullRequest.targetBranch || defaultBranch || activeBranch;
}
