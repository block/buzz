import {
  PROJECT_ISSUE_STATUS,
  PROJECT_ISSUE_STATUS_ORDER,
} from "../projectIssues.mjs";

const KNOWN_STATUSES = new Set(PROJECT_ISSUE_STATUS_ORDER);

export function normalizeProjectIssueBoardStatus(status) {
  return KNOWN_STATUSES.has(status) ? status : PROJECT_ISSUE_STATUS.BACKLOG;
}

function compareByLastTouched(left, right) {
  const updatedDifference = right.issue.updatedAt - left.issue.updatedAt;
  if (updatedDifference !== 0) return updatedDifference;

  const createdDifference = right.issue.createdAt - left.issue.createdAt;
  if (createdDifference !== 0) return createdDifference;

  return left.issue.title.localeCompare(right.issue.title);
}

export function groupProjectIssuesForBoard(issues) {
  const columns = PROJECT_ISSUE_STATUS_ORDER.map((status) => ({
    status,
    issues: [],
  }));
  const columnsByStatus = new Map(
    columns.map((column) => [column.status, column]),
  );

  for (const issue of issues) {
    const status = normalizeProjectIssueBoardStatus(issue.issue.status);
    columnsByStatus.get(status)?.issues.push(issue);
  }

  for (const column of columns) {
    column.issues.sort(compareByLastTouched);
  }

  return columns;
}
