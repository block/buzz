import assert from "node:assert/strict";
import test from "node:test";

import {
  projectReviewActivity,
  projectRightPanelScope,
  projectTaskActivity,
} from "./projectRightPanelContext.ts";

test("Git-backed tabs follow the selected branch", () => {
  for (const tab of ["overview", "files", "activity", "contributors"]) {
    assert.equal(projectRightPanelScope(tab), "branch", tab);
  }
});

test("collaborative tabs stay repository-wide", () => {
  for (const tab of ["issues", "prs", "channels"]) {
    assert.equal(projectRightPanelScope(tab), "repository", tab);
  }
});

test("task activity separates active and completed work", () => {
  assert.deepEqual(
    projectTaskActivity([
      { status: "Backlog" },
      { status: "In Progress" },
      { status: "Done" },
      { status: "Closed" },
    ]),
    { active: 2, completed: 2, total: 4 },
  );
});

test("review activity counts open drafts and merged reviews", () => {
  assert.deepEqual(
    projectReviewActivity([
      { status: "Open" },
      { status: "Draft" },
      { status: "Merged" },
      { status: "Closed" },
    ]),
    { merged: 1, open: 2, total: 4 },
  );
});
