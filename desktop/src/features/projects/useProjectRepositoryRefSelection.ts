import * as React from "react";

export function useProjectRepositoryRefSelection(input: {
  branchOptions: string[];
  defaultBranch: string | null;
  projectAvailable: boolean;
  projectPending: boolean;
  /** Identity of the repository the options describe. A switch to a
   * different repository resets the selection even when the new repository
   * happens to have a branch or tag with the same name — carrying `release`
   * from repo A into repo B would silently retarget files, actions, and
   * agent context at a ref the user never chose. */
  repositoryId: string | null;
  tags: Array<{ name: string }>;
}) {
  const [selectedBranch, setSelectedBranch] = React.useState<string | null>(
    null,
  );
  const [selectedTag, setSelectedTag] = React.useState<string | null>(null);
  const [selectionRepositoryId, setSelectionRepositoryId] = React.useState(
    input.repositoryId,
  );
  // Reset during render (not in an effect) so a repository switch can never
  // paint one frame with the previous repository's same-named selection.
  if (selectionRepositoryId !== input.repositoryId) {
    setSelectionRepositoryId(input.repositoryId);
    setSelectedBranch(null);
    setSelectedTag(null);
  }
  const staleSelection = selectionRepositoryId !== input.repositoryId;
  const activeBranch =
    (staleSelection ? null : selectedBranch) ??
    input.defaultBranch ??
    input.branchOptions[0] ??
    null;

  React.useEffect(() => {
    if (!input.projectAvailable) {
      if (input.projectPending) return;
      setSelectedBranch(null);
      setSelectedTag(null);
      return;
    }
    setSelectedBranch((currentBranch) => {
      if (currentBranch && input.branchOptions.includes(currentBranch)) {
        return currentBranch;
      }
      return input.defaultBranch ?? input.branchOptions[0] ?? null;
    });
    setSelectedTag((currentTag) => {
      if (currentTag && input.tags.some((tag) => tag.name === currentTag)) {
        return currentTag;
      }
      return null;
    });
  }, [
    input.branchOptions,
    input.defaultBranch,
    input.projectAvailable,
    input.projectPending,
    input.tags,
  ]);

  const selectBranch = React.useCallback((branch: string | null) => {
    setSelectedBranch(branch);
    setSelectedTag(null);
  }, []);
  const selectTag = React.useCallback((tag: string) => {
    setSelectedTag(tag);
  }, []);

  return {
    activeBranch,
    selectBranch,
    selectedTag: staleSelection ? null : selectedTag,
    selectTag,
  };
}
