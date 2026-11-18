import assert from "node:assert/strict";
import test from "node:test";

import { addProjectChannel } from "./useAddProjectChannel.ts";

const OWNER = "a".repeat(64);
const CREATED_CHANNEL = "22222222-2222-4222-8222-222222222222";

function makeProject(overrides = {}) {
  return {
    id: `30621:${OWNER}:platform`,
    dtag: "platform",
    name: "Platform",
    description: "",
    owner: OWNER,
    createdAt: 100,
    projectChannelId: "11111111-1111-4111-8111-111111111111",
    relatedChannelIds: [],
    status: "active",
    projectAddress: `30621:${OWNER}:platform`,
    primaryRepositoryAddress: null,
    repositoryAddresses: [],
    repositories: [],
    legacy: false,
    ...overrides,
  };
}

function makeLiveHead(createdAt, overrides = {}) {
  return {
    id: "f".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: createdAt,
    content: "",
    tags: [["d", "platform"]],
    sig: "0".repeat(128),
    ...overrides,
  };
}

function input() {
  return {
    name: "Engineering",
    project: makeProject(),
    visibility: "private",
  };
}

function dependencies(fetchEvents) {
  let createCalls = 0;
  return {
    deps: {
      applyAgents: async () => {},
      applyCanvas: async () => {},
      createChannel: async () => {
        createCalls += 1;
        throw new Error(
          "createChannel must not run before live-head preflight",
        );
      },
      fetchEvents,
    },
    createCalls: () => createCalls,
  };
}

test("addProjectChannel does not create a channel when the live project head is stale", async () => {
  const harness = dependencies(async () => [makeLiveHead(101)]);

  await assert.rejects(
    addProjectChannel(input(), harness.deps),
    /updated by another session/,
  );
  assert.equal(harness.createCalls(), 0);
});

test("addProjectChannel does not create a channel when the live project head is missing", async () => {
  const harness = dependencies(async () => []);

  await assert.rejects(
    addProjectChannel(input(), harness.deps),
    /Could not find this project on the relay/,
  );
  assert.equal(harness.createCalls(), 0);
});

test("addProjectChannel removes its channel and does not publish when the project changes during creation", async () => {
  const originalHead = makeLiveHead(100);
  const concurrentHead = makeLiveHead(101, {
    id: "e".repeat(64),
    tags: [
      ["d", "platform"],
      ["buzz-related-channel", "33333333-3333-4333-8333-333333333333"],
    ],
  });
  const fetchResults = [[originalHead], [concurrentHead]];
  const deleted = [];
  let publishCalls = 0;

  await assert.rejects(
    addProjectChannel(input(), {
      applyAgents: async () => {},
      applyCanvas: async () => {},
      createChannel: async () => ({ id: CREATED_CHANNEL }),
      deleteChannel: async (channelId) => deleted.push(channelId),
      fetchEvents: async () => fetchResults.shift() ?? [],
      publishOwnerAnnouncement: async () => {
        publishCalls += 1;
        throw new Error("must not publish a stale project replacement");
      },
    }),
    /updated by another session while the channel was being created/,
  );

  assert.deepEqual(deleted, [CREATED_CHANNEL]);
  assert.equal(publishCalls, 0);
});

test("addProjectChannel removes its channel when project publication fails", async () => {
  const liveHead = makeLiveHead(100);
  const deleted = [];
  let fetchCalls = 0;

  await assert.rejects(
    addProjectChannel(input(), {
      applyAgents: async () => {},
      applyCanvas: async () => {},
      createChannel: async () => ({ id: CREATED_CHANNEL }),
      deleteChannel: async (channelId) => deleted.push(channelId),
      fetchEvents: async () => {
        fetchCalls += 1;
        return [liveHead];
      },
      publishOwnerAnnouncement: async () => {
        throw new Error("publication failed");
      },
    }),
    /publication failed/,
  );

  assert.equal(fetchCalls, 2);
  assert.deepEqual(deleted, [CREATED_CHANNEL]);
});
