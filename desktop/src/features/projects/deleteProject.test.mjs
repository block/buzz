import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRepositoryDeletionTemplate,
  canDeleteRepository,
  deleteProject,
  deleteRepository,
} from "./projectDeletion.ts";

const OWNER = "a".repeat(64);
const VIEWER = "b".repeat(64);
const PROJECT_ADDRESS = `30621:${OWNER}:platform`;
const project = {
  createdAt: 50,
  description: "",
  dtag: "platform",
  id: PROJECT_ADDRESS,
  legacy: false,
  name: "Platform",
  owner: OWNER,
  primaryRepositoryAddress: `30617:${OWNER}:repo`,
  projectAddress: PROJECT_ADDRESS,
  projectChannelId: null,
  repositories: [],
  repositoryAddresses: [`30617:${OWNER}:repo`],
  status: "active",
};

const REPO_ADDRESS = `30617:${OWNER}:repo`;
const repository = {
  cloneUrls: [],
  contributors: [],
  createdAt: 50,
  defaultBranch: "main",
  description: "",
  dtag: "repo",
  id: REPO_ADDRESS,
  name: "Repo",
  owner: OWNER,
  repoAddress: REPO_ADDRESS,
  status: "active",
  webUrl: null,
};

function event(overrides = {}) {
  return {
    id: "1".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 75,
    content: "",
    tags: [["d", "platform"]],
    ...overrides,
  };
}

function repoEvent(overrides = {}) {
  return {
    id: "2".repeat(64),
    kind: 30617,
    pubkey: OWNER,
    created_at: 75,
    content: "",
    tags: [["d", "repo"]],
    ...overrides,
  };
}

test("deleteProject lets the relay authorize an agent owner and tombstones only the project", async () => {
  const calls = [];
  await deleteProject(project, {
    fetchEvents: async (filter) => {
      calls.push(["fetch", filter]);
      return calls.filter(([type]) => type === "fetch").length === 1
        ? [event()]
        : [];
    },
    nowSeconds: () => 74,
    signEvent: async (template) => {
      calls.push(["sign", template]);
      return event({
        pubkey: VIEWER,
        kind: template.kind,
        content: template.content,
        created_at: template.createdAt,
        tags: template.tags,
      });
    },
    publishEvent: async (signed) => {
      calls.push(["publish", signed]);
    },
  });

  assert.deepEqual(calls[0][1], {
    kinds: [30621],
    authors: [OWNER],
    "#d": ["platform"],
    limit: 1,
  });
  assert.deepEqual(calls[1][1].tags, [["a", PROJECT_ADDRESS]]);
  assert.equal(calls[1][1].createdAt, 76);
  assert.equal(calls[2][1].pubkey, VIEWER);
  assert.deepEqual(calls[3][1], calls[0][1]);
  assert.equal(calls[1][1].content, "Delete project Platform");
});

test("deleteProject builds a one-coordinate tombstone", async () => {
  const calls = [];
  await deleteProject(project, {
    fetchEvents: async () => (calls.length === 0 ? [event()] : []),
    signEvent: async (template) => {
      calls.push(template);
      return event({
        kind: template.kind,
        content: template.content,
        created_at: template.createdAt,
        tags: template.tags,
      });
    },
    publishEvent: async () => {},
  });

  assert.equal(calls[0].kind, 5);
  assert.deepEqual(calls[0].tags, [["a", PROJECT_ADDRESS]]);
});

test("deleteProject fails closed when the live project head is missing", async () => {
  await assert.rejects(
    deleteProject(project, { fetchEvents: async () => [] }),
    /Could not find this project on the relay/,
  );
});

test("deleteProject reports a concurrent replacement that survives", async () => {
  let fetchCount = 0;
  await assert.rejects(
    deleteProject(project, {
      fetchEvents: async () => {
        fetchCount += 1;
        return [event({ created_at: fetchCount === 1 ? 75 : 77 })];
      },
      nowSeconds: () => 74,
      signEvent: async (template) =>
        event({
          pubkey: VIEWER,
          kind: template.kind,
          created_at: template.createdAt,
          tags: template.tags,
        }),
      publishEvent: async () => {},
    }),
    /updated while it was being deleted/,
  );
});

test("deleteProject reports uncertain outcome when publish acknowledgement is lost", async () => {
  await assert.rejects(
    deleteProject(project, {
      fetchEvents: async () => [event()],
      signEvent: async (template) =>
        event({
          kind: template.kind,
          created_at: template.createdAt,
          tags: template.tags,
        }),
      publishEvent: async (_event, timeoutMessage) => {
        throw new Error(timeoutMessage);
      },
    }),
    /Could not confirm whether the project was deleted\. Projects were refreshed\./,
  );
});

test("canDeleteRepository authorizes owner and agent owner", () => {
  assert.equal(canDeleteRepository({ owner: OWNER }, OWNER), true);
  assert.equal(canDeleteRepository({ owner: OWNER }, VIEWER), false);
  assert.equal(canDeleteRepository({ owner: OWNER }, undefined), false);
  assert.equal(
    canDeleteRepository({ owner: OWNER }, VIEWER, {
      [OWNER]: {
        agentNip05: null,
        agentPubkey: null,
        agentSource: null,
        displayName: null,
        nip05: null,
        ownerPubkey: VIEWER,
        pictureUrl: null,
        pubkey: OWNER,
        username: null,
      },
    }),
    true,
  );
});

test("buildRepositoryDeletionTemplate builds a one-coordinate tombstone", () => {
  const template = buildRepositoryDeletionTemplate(
    repository,
    { created_at: 75 },
    74,
  );
  assert.equal(template.kind, 5);
  assert.deepEqual(template.tags, [["a", REPO_ADDRESS]]);
  assert.equal(template.createdAt, 76);
  assert.equal(template.content, "Delete repository Repo");
});

test("deleteRepository deletes repository and verifies removal", async () => {
  const calls = [];
  await deleteRepository(repository, {
    fetchEvents: async (filter) => {
      calls.push(["fetch", filter]);
      return calls.filter(([type]) => type === "fetch").length === 1
        ? [repoEvent()]
        : [];
    },
    nowSeconds: () => 74,
    signEvent: async (template) => {
      calls.push(["sign", template]);
      return repoEvent({
        pubkey: VIEWER,
        kind: template.kind,
        content: template.content,
        created_at: template.createdAt,
        tags: template.tags,
      });
    },
    publishEvent: async (signed) => {
      calls.push(["publish", signed]);
    },
  });

  assert.deepEqual(calls[0][1], {
    kinds: [30617],
    authors: [OWNER],
    "#d": ["repo"],
    limit: 1,
  });
  assert.deepEqual(calls[1][1].tags, [["a", REPO_ADDRESS]]);
  assert.equal(calls[1][1].createdAt, 76);
  assert.equal(calls[2][1].pubkey, VIEWER);
  assert.deepEqual(calls[3][1], calls[0][1]);
  assert.equal(calls[1][1].content, "Delete repository Repo");
});

test("deleteRepository fails closed when the live repository head is missing", async () => {
  await assert.rejects(
    deleteRepository(repository, { fetchEvents: async () => [] }),
    /Could not find this repository on the relay/,
  );
});

test("deleteRepository reports a concurrent replacement that survives", async () => {
  let fetchCount = 0;
  await assert.rejects(
    deleteRepository(repository, {
      fetchEvents: async () => {
        fetchCount += 1;
        return [repoEvent({ created_at: fetchCount === 1 ? 75 : 77 })];
      },
      nowSeconds: () => 74,
      signEvent: async (template) =>
        repoEvent({
          pubkey: VIEWER,
          kind: template.kind,
          created_at: template.createdAt,
          tags: template.tags,
        }),
      publishEvent: async () => {},
    }),
    /updated while it was being deleted/,
  );
});

test("deleteRepository reports uncertain outcome when publish acknowledgement is lost", async () => {
  await assert.rejects(
    deleteRepository(repository, {
      fetchEvents: async () => [repoEvent()],
      signEvent: async (template) =>
        repoEvent({
          kind: template.kind,
          created_at: template.createdAt,
          tags: template.tags,
        }),
      publishEvent: async (_event, timeoutMessage) => {
        throw new Error(timeoutMessage);
      },
    }),
    /Could not confirm whether the repository was deleted\. Repositories were refreshed\./,
  );
});
