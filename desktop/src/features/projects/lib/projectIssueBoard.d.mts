import type { ProjectIssueListItem } from "@/features/projects/hooks";
import type { ProjectIssueStatus } from "@/features/projects/projectIssues.mjs";

export type ProjectIssueBoardColumn = {
  status: ProjectIssueStatus;
  issues: ProjectIssueListItem[];
};

export function normalizeProjectIssueBoardStatus(
  status: string,
): ProjectIssueStatus;
export function groupProjectIssuesForBoard(
  issues: ProjectIssueListItem[],
): ProjectIssueBoardColumn[];
