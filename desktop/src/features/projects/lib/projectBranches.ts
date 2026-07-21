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
  localBranch?: string | null,
): string[] {
  return [
    ...new Set(
      [...remoteBranches, localBranch].filter((branch): branch is string =>
        Boolean(branch),
      ),
    ),
  ];
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
