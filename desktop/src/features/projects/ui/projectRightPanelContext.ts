import type {
  ProjectIssue,
  ProjectPullRequest,
} from "@/features/projects/hooks";

export type ProjectRightPanelScope = "branch" | "repository";

const BRANCH_SCOPED_TABS = new Set([
  "activity",
  "contributors",
  "files",
  "overview",
]);

/** Identifies whether a project tab follows the selected Git ref. */
export function projectRightPanelScope(
  activeTab: string,
): ProjectRightPanelScope {
  return BRANCH_SCOPED_TABS.has(activeTab) ? "branch" : "repository";
}

export function projectTaskActivity(issues: ProjectIssue[]) {
  const completed = issues.filter(
    (issue) => issue.status === "Done" || issue.status === "Closed",
  ).length;
  return {
    active: issues.length - completed,
    completed,
    total: issues.length,
  };
}

export function projectReviewActivity(pullRequests: ProjectPullRequest[]) {
  return {
    merged: pullRequests.filter(
      (pullRequest) => pullRequest.status === "Merged",
    ).length,
    open: pullRequests.filter(
      (pullRequest) =>
        pullRequest.status === "Open" || pullRequest.status === "Draft",
    ).length,
    total: pullRequests.length,
  };
}
