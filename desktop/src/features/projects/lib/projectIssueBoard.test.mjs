import assert from "node:assert/strict";
import { test } from "node:test";

import {
  groupProjectIssuesForBoard,
  normalizeProjectIssueBoardStatus,
} from "./projectIssueBoard.mjs";
import {
  PROJECT_ISSUE_STATUS,
  PROJECT_ISSUE_STATUS_ORDER,
} from "../projectIssues.mjs";

function item({
  id,
  status,
  updatedAt,
  createdAt = updatedAt - 10,
  title = id,
}) {
  return {
    project: { id: `project-${id}` },
    issue: { id, status, updatedAt, createdAt, title },
  };
}

test("board columns follow the canonical issue status order", () => {
  const columns = groupProjectIssuesForBoard(
    PROJECT_ISSUE_STATUS_ORDER.map((status, index) =>
      item({ id: `${index}`, status, updatedAt: index }),
    ),
  );

  assert.deepEqual(
    columns.map((column) => column.status),
    PROJECT_ISSUE_STATUS_ORDER,
  );
  assert.deepEqual(
    columns.map((column) => column.issues[0]?.issue.status),
    PROJECT_ISSUE_STATUS_ORDER,
  );
});

test("issues sort by last touched without mutating the source array", () => {
  const issues = [
    item({ id: "older", status: "Backlog", updatedAt: 10 }),
    item({ id: "newer", status: "Backlog", updatedAt: 30 }),
    item({ id: "middle", status: "Backlog", updatedAt: 20 }),
  ];
  const sourceOrder = issues.map(({ issue }) => issue.id);

  const backlog = groupProjectIssuesForBoard(issues).find(
    (column) => column.status === PROJECT_ISSUE_STATUS.BACKLOG,
  );

  assert.deepEqual(
    backlog.issues.map(({ issue }) => issue.id),
    ["newer", "middle", "older"],
  );
  assert.deepEqual(
    issues.map(({ issue }) => issue.id),
    sourceOrder,
  );
});

test("unknown statuses remain visible in Backlog", () => {
  const unknown = item({
    id: "future",
    status: "Future status",
    updatedAt: 10,
  });
  const columns = groupProjectIssuesForBoard([unknown]);

  assert.equal(
    normalizeProjectIssueBoardStatus(unknown.issue.status),
    PROJECT_ISSUE_STATUS.BACKLOG,
  );
  assert.equal(
    columns.find((column) => column.status === PROJECT_ISSUE_STATUS.BACKLOG)
      .issues[0],
    unknown,
  );
});

test("empty input keeps valid empty columns", () => {
  const columns = groupProjectIssuesForBoard([]);

  assert.equal(columns.length, PROJECT_ISSUE_STATUS_ORDER.length);
  assert.ok(columns.every((column) => column.issues.length === 0));
});

test("Done and Closed remain separate", () => {
  const columns = groupProjectIssuesForBoard([
    item({ id: "done", status: "Done", updatedAt: 20 }),
    item({ id: "closed", status: "Closed", updatedAt: 10 }),
  ]);

  assert.deepEqual(
    columns
      .filter((column) => column.issues.length > 0)
      .map((column) => [column.status, column.issues[0].issue.id]),
    [
      [PROJECT_ISSUE_STATUS.DONE, "done"],
      [PROJECT_ISSUE_STATUS.CLOSED, "closed"],
    ],
  );
});
