import assert from "node:assert/strict";
import { test } from "node:test";

import {
  collapseProjectRelatedChannelRows,
  collectProjectRelatedChannelRows,
  listProjectBoundChannels,
  listProjectChildChannels,
  projectRelatedChannelRowKey,
  uniqueProjectRelatedChannelCount,
} from "./projectRelatedChannels.ts";

const CHANNEL_A = "11111111-1111-4111-8111-111111111111";
const CHANNEL_B = "22222222-2222-4222-8222-222222222222";

function makeProject(overrides = {}) {
  return {
    id: "project-buzz",
    name: "buzz",
    projectChannelId: null,
    repositories: [],
    ...overrides,
  };
}

function makeRepository(overrides = {}) {
  return {
    id: "repo-buzz",
    name: "buzz",
    channelId: CHANNEL_A,
    ...overrides,
  };
}

test("collects one row per repository channel binding", () => {
  const rows = collectProjectRelatedChannelRows([
    makeProject({
      repositories: [
        makeRepository(),
        makeRepository({
          id: "repo-relay",
          name: "relay-tools",
          channelId: CHANNEL_A,
        }),
      ],
    }),
    makeProject({
      id: "project-design",
      name: "design-system",
      repositories: [
        makeRepository({
          id: "repo-design",
          name: "design-system",
          channelId: CHANNEL_A,
        }),
      ],
    }),
  ]);

  assert.deepEqual(rows, [
    {
      channelId: CHANNEL_A,
      projectId: "project-buzz",
      projectName: "buzz",
      repositoryId: "repo-buzz",
      repositoryName: "buzz",
    },
    {
      channelId: CHANNEL_A,
      projectId: "project-buzz",
      projectName: "buzz",
      repositoryId: "repo-relay",
      repositoryName: "relay-tools",
    },
    {
      channelId: CHANNEL_A,
      projectId: "project-design",
      projectName: "design-system",
      repositoryId: "repo-design",
      repositoryName: "design-system",
    },
  ]);
  assert.equal(uniqueProjectRelatedChannelCount([]), 0);
  assert.equal(
    uniqueProjectRelatedChannelCount([
      makeProject({
        repositories: [
          makeRepository(),
          makeRepository({ id: "repo-relay", channelId: CHANNEL_A }),
        ],
      }),
      makeProject({
        id: "project-design",
        repositories: [makeRepository({ id: "repo-design" })],
      }),
    ]),
    1,
  );
  assert.equal(
    uniqueProjectRelatedChannelCount([
      makeProject({
        projectChannelId: CHANNEL_B,
        repositories: [makeRepository()],
      }),
    ]),
    2,
  );
});

test("collapses repositories sharing one project channel", () => {
  const rows = collectProjectRelatedChannelRows([
    makeProject({
      repositories: [
        makeRepository({ name: "web" }),
        makeRepository({ id: "repo-mobile", name: "mobile" }),
        makeRepository({
          channelId: CHANNEL_B,
          id: "repo-relay",
          name: "relay",
        }),
      ],
    }),
  ]);

  assert.deepEqual(collapseProjectRelatedChannelRows(rows), [
    {
      channelId: CHANNEL_A,
      projectId: "project-buzz",
      projectName: "buzz",
      repositoryNames: ["web", "mobile"],
    },
    {
      channelId: CHANNEL_B,
      projectId: "project-buzz",
      projectName: "buzz",
      repositoryNames: ["relay"],
    },
  ]);
});

test("keeps a project channel only when no repository in that project shares it", () => {
  assert.deepEqual(
    collectProjectRelatedChannelRows([
      makeProject({
        projectChannelId: CHANNEL_A,
        repositories: [makeRepository({ channelId: CHANNEL_A })],
      }),
    ]),
    [
      {
        channelId: CHANNEL_A,
        projectId: "project-buzz",
        projectName: "buzz",
        repositoryId: "repo-buzz",
        repositoryName: "buzz",
      },
    ],
  );

  assert.deepEqual(
    collectProjectRelatedChannelRows([
      makeProject({
        projectChannelId: CHANNEL_B,
        repositories: [makeRepository({ channelId: CHANNEL_A })],
      }),
    ]),
    [
      {
        channelId: CHANNEL_A,
        projectId: "project-buzz",
        projectName: "buzz",
        repositoryId: "repo-buzz",
        repositoryName: "buzz",
      },
      {
        channelId: CHANNEL_B,
        projectId: "project-buzz",
        projectName: "buzz",
        repositoryId: null,
        repositoryName: null,
      },
    ],
  );
});

test("skips blank channel ids", () => {
  assert.deepEqual(
    collectProjectRelatedChannelRows([
      makeProject({
        projectChannelId: "   ",
        repositories: [makeRepository({ channelId: "" })],
      }),
    ]),
    [],
  );
});

test("row keys distinguish project-level bindings from repository bindings", () => {
  assert.equal(
    projectRelatedChannelRowKey({
      channelId: CHANNEL_A,
      projectId: "project-buzz",
      projectName: "buzz",
      repositoryId: null,
      repositoryName: null,
    }),
    `${CHANNEL_A}:project-buzz:project`,
  );
  assert.equal(
    projectRelatedChannelRowKey({
      channelId: CHANNEL_A,
      projectId: "project-buzz",
      projectName: "buzz",
      repositoryId: "repo-buzz",
      repositoryName: "buzz",
    }),
    `${CHANNEL_A}:project-buzz:repo-buzz`,
  );
});

test("listProjectBoundChannels puts the home channel first", () => {
  assert.deepEqual(
    listProjectBoundChannels(
      makeProject({
        projectChannelId: CHANNEL_B,
        repositories: [
          makeRepository({ channelId: CHANNEL_A }),
          makeRepository({
            id: "repo-relay",
            name: "relay-tools",
            channelId: CHANNEL_A,
          }),
        ],
      }),
    ),
    [
      {
        channelId: CHANNEL_B,
        repositoryId: null,
        role: "home",
      },
      {
        channelId: CHANNEL_A,
        repositoryId: "repo-buzz",
        role: "related",
      },
    ],
  );
});

test("listProjectBoundChannels omits a repository channel that is the home channel", () => {
  assert.deepEqual(
    listProjectBoundChannels(
      makeProject({
        projectChannelId: CHANNEL_A,
        repositories: [makeRepository({ channelId: CHANNEL_A })],
      }),
    ),
    [
      {
        channelId: CHANNEL_A,
        repositoryId: null,
        role: "home",
      },
    ],
  );
});

test("listProjectBoundChannels is empty when nothing is bound", () => {
  assert.deepEqual(listProjectBoundChannels(makeProject()), []);
});

test("listProjectBoundChannels includes extra related channels after home", () => {
  const related = "33333333-3333-4333-8333-333333333333";
  assert.deepEqual(
    listProjectBoundChannels(
      makeProject({
        projectChannelId: CHANNEL_B,
        relatedChannelIds: [related],
        repositories: [makeRepository({ channelId: CHANNEL_A })],
      }),
    ),
    [
      {
        channelId: CHANNEL_B,
        repositoryId: null,
        role: "home",
      },
      {
        channelId: related,
        repositoryId: null,
        role: "related",
      },
      {
        channelId: CHANNEL_A,
        repositoryId: "repo-buzz",
        role: "related",
      },
    ],
  );
});

test("listProjectChildChannels omits the home channel", () => {
  const related = "33333333-3333-4333-8333-333333333333";
  assert.deepEqual(
    listProjectChildChannels(
      makeProject({
        projectChannelId: CHANNEL_B,
        relatedChannelIds: [related],
        repositories: [makeRepository({ channelId: CHANNEL_A })],
      }),
    ),
    [
      {
        channelId: related,
        repositoryId: null,
        role: "related",
      },
      {
        channelId: CHANNEL_A,
        repositoryId: "repo-buzz",
        role: "related",
      },
    ],
  );
});

test("collectProjectRelatedChannelRows includes extra related channels", () => {
  const related = "33333333-3333-4333-8333-333333333333";
  const rows = collectProjectRelatedChannelRows([
    makeProject({
      projectChannelId: CHANNEL_B,
      relatedChannelIds: [related, CHANNEL_A],
      repositories: [makeRepository()],
    }),
  ]);
  assert.equal(
    rows.some((row) => row.channelId === related && row.repositoryId == null),
    true,
  );
  assert.equal(
    uniqueProjectRelatedChannelCount([
      makeProject({
        projectChannelId: CHANNEL_B,
        relatedChannelIds: [related],
        repositories: [makeRepository()],
      }),
    ]),
    3,
  );
});
