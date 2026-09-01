import assert from "node:assert/strict";
import test from "node:test";

import { ProjectRevisionPublicationError } from "./projectRelatedChannelRevision.ts";
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
    baseRevisionId: "f".repeat(64),
    effectiveRevisionId: "f".repeat(64),
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
      publishRevision: async () => {
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
      publishRevision: async () => {
        throw new Error("publication failed");
      },
    }),
    /publication failed/,
  );

  assert.equal(fetchCalls, 2);
  assert.deepEqual(deleted, [CREATED_CHANNEL]);
});

test("addProjectChannel keeps the channel when a lost acknowledgement is reconciled", async () => {
  const liveHead = makeLiveHead(100);
  const revision = {
    id: "e".repeat(64),
    kind: 47001,
    pubkey: "b".repeat(64),
    created_at: 101,
    content: "",
    tags: [],
  };
  const deleted = [];
  let liveFetches = 0;

  const result = await addProjectChannel(input(), {
    applyAgents: async () => {},
    applyCanvas: async () => {},
    createChannel: async () => ({ id: CREATED_CHANNEL }),
    deleteChannel: async (channelId) => deleted.push(channelId),
    fetchEvents: async () => {
      liveFetches += 1;
      return [liveHead];
    },
    fetchRevisionHeads: async () => [revision],
    publishRevision: async () => {
      throw new ProjectRevisionPublicationError(
        revision,
        new Error("confirmation timed out"),
      );
    },
  });

  assert.equal(liveFetches, 2);
  assert.deepEqual(deleted, []);
  assert.deepEqual(result.project.relatedChannelIds, [CREATED_CHANNEL]);
  assert.equal(result.project.effectiveRevisionId, revision.id);
});

test("addProjectChannel cleans up when reconciliation confirms rejection", async () => {
  const liveHead = makeLiveHead(100);
  const revision = {
    id: "e".repeat(64),
    kind: 47001,
    pubkey: "b".repeat(64),
    created_at: 101,
    content: "",
    tags: [],
  };
  const deleted = [];

  await assert.rejects(
    addProjectChannel(input(), {
      applyAgents: async () => {},
      applyCanvas: async () => {},
      createChannel: async () => ({ id: CREATED_CHANNEL }),
      deleteChannel: async (channelId) => deleted.push(channelId),
      fetchEvents: async () => [liveHead],
      fetchRevisionHeads: async () => [],
      publishRevision: async () => {
        throw new ProjectRevisionPublicationError(
          revision,
          new Error("relay rejected revision"),
        );
      },
    }),
    /relay rejected revision/,
  );

  assert.deepEqual(deleted, [CREATED_CHANNEL]);
});

test("addProjectChannel preserves the channel when reconciliation is unavailable", async () => {
  const liveHead = makeLiveHead(100);
  const revision = {
    id: "e".repeat(64),
    kind: 47001,
    pubkey: "b".repeat(64),
    created_at: 101,
    content: "",
    tags: [],
  };
  const deleted = [];

  await assert.rejects(
    addProjectChannel(input(), {
      applyAgents: async () => {},
      applyCanvas: async () => {},
      createChannel: async () => ({ id: CREATED_CHANNEL }),
      deleteChannel: async (channelId) => deleted.push(channelId),
      fetchEvents: async () => [liveHead],
      fetchRevisionHeads: async () => {
        throw new Error("relay unavailable");
      },
      publishRevision: async () => {
        throw new ProjectRevisionPublicationError(
          revision,
          new Error("confirmation timed out"),
        );
      },
    }),
    /new channel was kept/,
  );

  assert.deepEqual(deleted, []);
});

test("addProjectChannel publishes an actor-signed revision and advances the local CAS head", async () => {
  const liveHead = makeLiveHead(100);
  const revision = {
    id: "e".repeat(64),
    kind: 47001,
    pubkey: "b".repeat(64),
    created_at: 90,
    content: "",
    tags: [],
  };
  const calls = [];
  const result = await addProjectChannel(input(), {
    applyAgents: async () => {},
    applyCanvas: async () => {},
    createChannel: async () => ({ id: CREATED_CHANNEL }),
    fetchEvents: async () => [liveHead],
    publishRevision: async (...args) => {
      calls.push(args);
      return revision;
    },
  });

  assert.deepEqual(calls, [
    [input().project, CREATED_CHANNEL, "add-related-channel"],
  ]);
  assert.deepEqual(result.project.relatedChannelIds, [CREATED_CHANNEL]);
  assert.equal(result.project.effectiveRevisionId, revision.id);
  assert.equal(result.project.createdAt, 100);
});
