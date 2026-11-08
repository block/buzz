import assert from "node:assert/strict";
import test from "node:test";

import { buildProjectsActivityDigest } from "./projectsActivityDigest.ts";

const NOW = 2_000_000_000;

test("summarizes recent activity in a short highlighted sentence", () => {
  const project = { id: "project-a" };
  const digest = buildProjectsActivityDigest({
    issues: [
      { project, issue: { createdAt: NOW - 60 } },
      { project, issue: { createdAt: NOW - 120 } },
    ],
    nowSeconds: NOW,
    projects: [project],
    pullRequests: [{ project, pullRequest: { createdAt: NOW - 180 } }],
    snapshots: {
      "project-a": {
        commits: [
          { timestamp: NOW - 30 },
          { timestamp: NOW - 90 },
          { timestamp: NOW - 8 * 24 * 60 * 60 },
        ],
      },
    },
  });

  assert.equal(digest.prefix, "This week:");
  assert.deepEqual(digest.highlights, [
    "2 new commits",
    "2 tasks opened",
    "1 review opened",
    "1 active project",
  ]);
  assert.ok(
    `${digest.prefix} ${digest.highlights.join(", ")}${digest.suffix}`.split(
      /\s+/,
    ).length <= 30,
  );
});

test("falls back to current totals when no recent activity is loaded", () => {
  const digest = buildProjectsActivityDigest({
    issues: [],
    nowSeconds: NOW,
    projects: [{ id: "a" }, { id: "b" }],
    pullRequests: [],
    summaries: {
      a: { issueCount: 4, prCount: 2 },
      b: { issueCount: 1, prCount: 3 },
    },
  });

  assert.equal(digest.prefix, "Currently tracking");
  assert.deepEqual(digest.highlights, ["2 projects", "5 tasks", "5 reviews"]);
});
