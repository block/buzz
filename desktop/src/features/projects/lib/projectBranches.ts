const BRANCH_CHARACTERS = /^[A-Za-z0-9/_.-]+$/;

/** Normalize a branch name using the native command's conservative rules. */
export function normalizeProjectBranchName(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed.startsWith("refs/") && !trimmed.startsWith("refs/heads/")) {
    return null;
  }
  const branch = trimmed.replace(/^refs\/heads\//, "");
  if (
    !branch ||
    branch.startsWith("-") ||
    branch.startsWith("/") ||
    branch.endsWith("/") ||
    branch.endsWith(".") ||
    branch.endsWith(".lock") ||
    branch.includes("..") ||
    branch.includes("//") ||
    !BRANCH_CHARACTERS.test(branch) ||
    branch.split("/").some((component) => component.startsWith("."))
  ) {
    return null;
  }
  return branch;
}

export function projectBranchNameError(
  value: string,
  existingBranches: string[],
): string | null {
  const branch = normalizeProjectBranchName(value);
  if (!branch) return "Enter a valid Git branch name.";
  if (existingBranches.includes(branch)) {
    return "A branch with this name already exists.";
  }
  return null;
}

export function projectBranchOptions(
  remoteBranches: string[],
  localBranches: string[] = [],
): string[] {
  return [...new Set([...remoteBranches, ...localBranches].filter(Boolean))];
}

export function projectBranchOptionsFromSync(
  remoteBranches: string[],
  syncStatus?: {
    localBranch: string | null;
    localBranches: string[];
    localHead: string | null;
  },
): string[] {
  const localBranches =
    syncStatus?.localBranches ??
    (syncStatus?.localHead && syncStatus.localBranch
      ? [syncStatus.localBranch]
      : []);
  return projectBranchOptions(remoteBranches, localBranches);
}

export function projectBranchCreationReason(input: {
  activeBranch: string | null;
  activeBranchCommit: string | null;
  localHead?: string | null;
}): string | null {
  if (!input.activeBranch) return "Choose a branch first.";
  if (input.activeBranchCommit) return null;
  return input.localHead
    ? `Push the first local commit to ${input.activeBranch} before creating another branch.`
    : "Create the repository's first commit before creating another branch.";
}

/** Resolve a usable default branch when a repository advertises a stale HEAD. */
export function resolveProjectDefaultBranch(
  announcedBranch: string,
  repoState?: {
    branches: Array<{ name: string }>;
    head: string | null;
  } | null,
): string {
  if (!repoState || repoState.branches.length === 0) {
    return repoState?.head ?? announcedBranch;
  }
  const published = new Set(repoState.branches.map((branch) => branch.name));
  if (repoState.head && published.has(repoState.head)) return repoState.head;
  if (published.has(announcedBranch)) return announcedBranch;
  if (published.has("main")) return "main";
  if (published.has("master")) return "master";
  return repoState.branches[0]?.name ?? announcedBranch;
}

export function projectBranchManagementState(input: {
  activeBranch: string | null;
  defaultBranch: string | null;
  branches: Array<{ name: string; commit: string }>;
  remoteBranch?: string | null;
  remoteHead?: string | null;
  snapshotCommit?: string | null;
  hasOpenPullRequest: boolean;
}) {
  const activeRemoteBranch =
    input.branches.find((branch) => branch.name === input.activeBranch) ?? null;
  const activeBranchCommit =
    activeRemoteBranch?.commit ??
    (input.remoteBranch === input.activeBranch ? input.remoteHead : null) ??
    input.snapshotCommit ??
    null;
  const deleteBranchReason = !input.activeBranch
    ? "Choose a branch first."
    : input.activeBranch === input.defaultBranch
      ? "The repository's default branch cannot be deleted."
      : !activeRemoteBranch
        ? "Only a published remote branch can be deleted."
        : input.hasOpenPullRequest
          ? "Close the branch's pull request before deleting it."
          : null;
  return { activeBranchCommit, activeRemoteBranch, deleteBranchReason };
}
