/**
 * The hovered collapse-branch range, or null when nothing is hovered. Rows whose
 * index falls inside `(startIndex, endIndex]` belong to the branch.
 */
export type HighlightedThreadBranch = {
  id: string;
  depth: number;
  startIndex: number;
  endIndex: number;
} | null;

/** Per-row branch-highlight flags derived from the hovered collapse branch. */
export type ThreadRowHighlight = {
  isBranchOwner: boolean;
  isInsideBranch: boolean;
  isDirectChild: boolean;
  lineDepths: number[] | undefined;
};

/**
 * Which highlight decorations a reply row shows for the hovered collapse branch.
 * Pure so the index-range logic is unit-testable without rendering the panel.
 */
export function selectThreadRowHighlight({
  branch,
  index,
  messageId,
  messageDepth,
  showGuides,
}: {
  branch: HighlightedThreadBranch;
  index: number;
  messageId: string;
  messageDepth: number;
  showGuides: boolean;
}): ThreadRowHighlight {
  const isBranchOwner = branch?.id === messageId;
  const isInsideBranch =
    branch != null && index > branch.startIndex && index <= branch.endIndex;
  const isDirectChild =
    isInsideBranch && branch != null && messageDepth === branch.depth + 1;
  const lineDepths =
    showGuides && isInsideBranch && branch ? [branch.depth] : undefined;
  return { isBranchOwner, isInsideBranch, isDirectChild, lineDepths };
}
