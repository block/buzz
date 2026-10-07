import assert from "node:assert/strict";
import { test } from "node:test";

import {
  collectProjectRelatedChannelRows,
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
