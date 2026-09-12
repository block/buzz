import assert from "node:assert/strict";
import test from "node:test";

import { buildProjectsViewAgentContextItems } from "./buildProjectsViewAgentContext.ts";

const repository = {
  description: "Relay and desktop source",
  name: "Buzz",
  repoAddress: "owner:buzz",
};
const project = {
  createdAt: 1,
  description: "Community platform",
  id: "owner:project",
  name: "Buzz Patrol",
  owner: "owner",
  projectChannelId: "project-channel",
  repositories: [
    { ...repository, channelId: "repository-channel", id: "repo" },
  ],
};
const issue = {
  content: "Fix the context",
  id: "issue-1",
  status: "Open",
  title: "Agent context",
};
const pullRequest = {
  content: "Includes visible rows",
  id: "review-1",
  status: "Open",
  title: "Expose overview data",
};
const base = {
  channels: [
    {
      description: "Repository discussion",
      id: "repository-channel",
      memberCount: 4,
      name: "buzz-dev",
    },
  ],
  issues: [],
  projects: [project],
  pullRequests: [],
  visibleIssues: [{ issue, project, repository }],
  visibleProjects: [project],
  visiblePullRequests: [{ project, pullRequest, repository }],
  visibleRepositories: [{ project, repository }],
};

for (const [filter, expected] of [
  ["all", "Buzz Patrol"],
  ["projects", "Buzz Patrol"],
  ["repositories", "Buzz"],
  ["issues", "Agent context"],
  ["prs", "Expose overview data"],
  ["channels", "#buzz-dev"],
]) {
  test(`builds ${filter} overview agent items`, () => {
    const items = buildProjectsViewAgentContextItems({ ...base, filter });
    assert.equal(items[0]?.title, expected);
    assert.ok(items[0]?.detail);
  });
}
