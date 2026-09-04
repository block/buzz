import assert from "node:assert/strict";
import { test } from "node:test";

import { projectsOverviewContext } from "./projectsOverviewContext.ts";

const CHANNEL_A = "11111111-1111-4111-8111-111111111111";
const CHANNEL_B = "22222222-2222-4222-8222-222222222222";
const OWNER = "a".repeat(64);
const TASK_AUTHOR = "b".repeat(64);
const REVIEW_AUTHOR = "c".repeat(64);
const BYSTANDER = "d".repeat(64);

function makeProject(overrides = {}) {
  return {
    id: "project-buzz",
    name: "buzz",
    owner: OWNER,
    projectChannelId: null,
    repositories: [
      {
        id: "repo-buzz",
        name: "buzz",
        owner: OWNER,
        channelId: CHANNEL_A,
        contributors: [BYSTANDER],
      },
    ],
    ...overrides,
  };
}

function makeIssue(status, author = TASK_AUTHOR) {
  return {
    author,
    comments: [],
    createdAt: 1_700_000_000,
    status,
  };
}

function makePullRequest(status, author = REVIEW_AUTHOR) {
  return {
    approvals: [],
    author,
    comments: [],
    commit: "1".repeat(40),
    createdAt: 1_700_086_400,
    initialCommit: "1".repeat(40),
    status,
    updates: [],
  };
}

test("activity pod shows workspace details without a create action", () => {
  const context = projectsOverviewContext({
    filter: "all",
    issues: [],
    projects: [
      makeProject(),
      makeProject({
        id: "project-relay",
        name: "relay",
        repositories: [
          { id: "repo-relay", name: "relay-tools", channelId: CHANNEL_B },
        ],
      }),
    ],
    pullRequests: [],
    summaries: {
      "project-buzz": { issueCount: 4, prCount: 2 },
      "project-relay": { issueCount: 1, prCount: 3 },
    },
  });

  assert.equal(context.title, "Activity");
  assert.equal(context.detailsTitle, "Details");
  assert.equal(context.action, null);
  assert.deepEqual(
    context.stats.map((stat) => [stat.label, stat.count]),
    [
      ["Projects", 2],
      ["Repositories", 2],
      ["Channels", 2],
      ["Tasks", 5],
      ["Reviews", 5],
    ],
  );
});

test("projects pod keeps create-project and drops task/review totals", () => {
  const context = projectsOverviewContext({
    filter: "projects",
    issues: [],
    projects: [makeProject(), makeProject({ id: "project-relay" })],
    pullRequests: [],
  });

  assert.equal(context.title, "Projects");
  assert.equal(context.action?.kind, "project");
  assert.deepEqual(
    context.stats.map((stat) => stat.label),
    ["Projects", "Repositories", "Channels"],
  );
});

test("repositories pod matches repository activity copy and add action", () => {
  const context = projectsOverviewContext({
    filter: "repositories",
    issues: [makeIssue("In Progress"), makeIssue("Done")],
    projects: [makeProject()],
    pullRequests: [makePullRequest("Open"), makePullRequest("Merged")],
  });

  assert.equal(context.title, "Repositories");
  assert.equal(context.detailsTitle, "Repository activity");
  assert.equal(context.action?.kind, "repository");
  assert.equal(context.action?.label, "Add repository");
  assert.deepEqual(
    context.stats.map((stat) => [stat.label, stat.count]),
    [
      ["Repositories", 1],
      ["Active tasks", 1],
      ["Open reviews", 1],
    ],
  );
});

test("channels pod titles itself and provides an add action", () => {
  const context = projectsOverviewContext({
    filter: "channels",
    issues: [],
    projects: [makeProject()],
    pullRequests: [],
  });

  assert.equal(context.title, "Channels");
  assert.equal(context.detailsTitle, "Details");
  assert.equal(context.action?.kind, "channel");
  assert.equal(context.action?.label, "Add channel");
  assert.deepEqual(
    context.stats.map((stat) => stat.label),
    ["Channels", "Projects", "Repositories"],
  );
});

test("repository-only read models count toward repository and channel context", () => {
  const explicitProject = makeProject();
  const repositoryOnly = makeProject({
    id: "repository-design-system",
    name: "design-system",
    repositories: [
      {
        id: "repo-design-system",
        name: "design-system",
        owner: REVIEW_AUTHOR,
        channelId: CHANNEL_B,
      },
    ],
  });
  const repositorySummaries = {
    "repo-design-system": {
      latestCommit: { author: REVIEW_AUTHOR },
    },
  };

  const repositories = projectsOverviewContext({
    filter: "repositories",
    issues: [],
    projectReadModels: [explicitProject, repositoryOnly],
    projects: [explicitProject],
    pullRequests: [],
    repositorySummaries,
  });
  const channels = projectsOverviewContext({
    filter: "channels",
    issues: [],
    projectReadModels: [explicitProject, repositoryOnly],
    projects: [explicitProject],
    pullRequests: [],
    repositorySummaries,
  });

  assert.deepEqual(
    repositories.stats.map((stat) => [stat.label, stat.count]),
    [
      ["Repositories", 2],
      ["Active tasks", 0],
      ["Open reviews", 0],
    ],
  );
  assert.deepEqual(repositories.people, [REVIEW_AUTHOR]);
  assert.deepEqual(
    channels.stats.map((stat) => [stat.label, stat.count]),
    [
      ["Channels", 2],
      ["Projects", 1],
      ["Repositories", 2],
    ],
  );
});

test("tasks pod breaks down active and completed work", () => {
  const context = projectsOverviewContext({
    filter: "issues",
    issues: [makeIssue("Triage"), makeIssue("Done"), makeIssue("Closed")],
    projects: [makeProject()],
    pullRequests: [],
  });

  assert.equal(context.title, "Tasks");
  assert.equal(context.action?.label, "Create task");
  assert.deepEqual(
    context.stats.map((stat) => [stat.label, stat.count]),
    [
      ["Tasks", 3],
      ["Active", 1],
      ["Completed", 2],
    ],
  );
});

test("reviews pod breaks down open and merged work", () => {
  const context = projectsOverviewContext({
    filter: "prs",
    issues: [],
    projects: [makeProject()],
    pullRequests: [
      makePullRequest("Open"),
      makePullRequest("Draft"),
      makePullRequest("Merged"),
      makePullRequest("Closed"),
    ],
  });

  assert.equal(context.title, "Reviews");
  assert.equal(context.detailsTitle, "Review activity");
  assert.equal(context.action?.label, "Create review");
  assert.deepEqual(
    context.stats.map((stat) => [stat.label, stat.count]),
    [
      ["Reviews", 4],
      ["Open", 2],
      ["Merged", 1],
    ],
  );
});

function peopleContext(filter, overrides = {}) {
  return projectsOverviewContext({
    filter,
    issues: [makeIssue("In Progress")],
    projects: [makeProject()],
    pullRequests: [makePullRequest("Open")],
    summaries: {
      "project-buzz": {
        latestCommit: { author: REVIEW_AUTHOR },
      },
    },
    ...overrides,
  });
}

test("people on activity, tasks, reviews, and repositories are actual actors", () => {
  const activity = peopleContext("all");
  const tasks = peopleContext("issues");
  const reviews = peopleContext("prs");
  const repositories = peopleContext("repositories");

  assert.deepEqual([...activity.people].sort(), [TASK_AUTHOR, REVIEW_AUTHOR]);
  assert.deepEqual(tasks.people, [TASK_AUTHOR]);
  assert.deepEqual(reviews.people, [REVIEW_AUTHOR]);
  assert.deepEqual(repositories.people, [REVIEW_AUTHOR]);
  assert.equal(activity.people.includes(BYSTANDER), false);
  assert.equal(activity.people.includes(OWNER), false);
});

test("listed repository contributors without commits stay off the repositories pod", () => {
  const context = peopleContext("repositories", {
    issues: [],
    pullRequests: [],
    summaries: {},
  });
  assert.deepEqual(context.people, []);
});

test("projects pod shows owners; channels hides people", () => {
  const projects = peopleContext("projects");
  const channels = peopleContext("channels");

  assert.deepEqual(projects.people, [OWNER]);
  assert.deepEqual(channels.people, []);
});
