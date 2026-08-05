import {
  normalizeRepositoryPath,
  parseRepositoryRef,
} from "../../../shared/lib/repositoryTarget.ts";

export type ProjectDetailSearch = {
  commitHash: string | undefined;
  pullRequestId: string | undefined;
  issueId: string | undefined;
  repositoryId: string | undefined;
  repoRef: string | undefined;
  repoPath: string | undefined;
};

export function validateProjectDetailSearch(
  search: Record<string, unknown>,
): ProjectDetailSearch {
  const commitHash =
    typeof search.commitHash === "string" ? search.commitHash : undefined;
  const pullRequestId =
    typeof search.pullRequestId === "string" ? search.pullRequestId : undefined;
  const issueId =
    typeof search.issueId === "string" ? search.issueId : undefined;
  const repositoryId =
    typeof search.repositoryId === "string" ? search.repositoryId : undefined;
  const parsedRef =
    typeof search.repoRef === "string"
      ? parseRepositoryRef(search.repoRef)
      : null;
  const repoPath =
    typeof search.repoPath === "string"
      ? normalizeRepositoryPath(search.repoPath)
      : null;
  const hasRepositoryTarget = Boolean(parsedRef && repoPath);

  return {
    commitHash,
    pullRequestId,
    issueId,
    repositoryId,
    repoRef: hasRepositoryTarget ? parsedRef?.value : undefined,
    repoPath: hasRepositoryTarget ? (repoPath ?? undefined) : undefined,
  };
}
