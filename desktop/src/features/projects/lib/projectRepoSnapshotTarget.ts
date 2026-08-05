import { parseRepositoryRef } from "../../../shared/lib/repositoryTarget.ts";

type PullRequestTarget = {
  cloneUrls: string[];
  id: string;
  commit: string | null;
};
type TagTarget = { name: string; commit: string };

export function projectRepoSnapshotCloneUrl(input: {
  projectCloneUrls: readonly string[];
  pullRequestCloneUrls?: readonly string[];
  repositoryTarget?: { ref: string; path: string } | null;
}): string | undefined {
  return input.repositoryTarget
    ? input.projectCloneUrls[0]
    : (input.pullRequestCloneUrls?.[0] ?? input.projectCloneUrls[0]);
}

export function projectRepoSnapshotTarget(input: {
  selectedBranch: string | null | undefined;
  projectDefaultBranch: string;
  pullRequest?: PullRequestTarget | null;
  tag?: TagTarget | null;
  repositoryRef?: string | null;
}) {
  const explicitRef = input.repositoryRef
    ? parseRepositoryRef(input.repositoryRef)
    : null;
  if (explicitRef?.kind === "branch") {
    return {
      defaultBranch: explicitRef.value,
      baseBranch: input.projectDefaultBranch,
      targetRef: `refs/heads/${explicitRef.value}`,
      targetCommit: null,
    };
  }
  if (explicitRef?.kind === "commit") {
    return {
      defaultBranch: input.selectedBranch ?? input.projectDefaultBranch,
      baseBranch: input.projectDefaultBranch,
      targetRef: null,
      targetCommit: explicitRef.value,
    };
  }
  return {
    defaultBranch: input.selectedBranch ?? input.projectDefaultBranch,
    baseBranch: input.projectDefaultBranch,
    targetRef: input.tag
      ? `refs/tags/${input.tag.name}`
      : input.pullRequest
        ? `refs/nostr/${input.pullRequest.id}`
        : null,
    targetCommit: input.tag?.commit ?? input.pullRequest?.commit ?? null,
  };
}
