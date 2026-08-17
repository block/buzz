function quoteShellArgument(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

/** Build user-visible recovery commands; callers decide whether to copy them. */
export function projectPullRequestConflictCommands({
  recoveryRef,
  targetBranch,
  targetRef,
}: {
  recoveryRef: string;
  targetBranch: string;
  targetRef: string;
}): string[] {
  const quotedTargetBranch = quoteShellArgument(targetBranch);
  const quotedTargetRef = quoteShellArgument(targetRef);
  return [
    'test -z "$(git status --porcelain=v1)" &&',
    `(git switch ${quotedTargetBranch} || git switch --create ${quotedTargetBranch} ${quotedTargetRef}) &&`,
    `git merge-base --is-ancestor HEAD ${quotedTargetRef} &&`,
    `git merge --ff-only ${quotedTargetRef} &&`,
    `git merge ${quoteShellArgument(recoveryRef)}`,
  ];
}
