import assert from "node:assert/strict";
import test from "node:test";

import { eventToProjectPullRequest } from "./projectPullRequests.mjs";

const OWNER = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const ATTACKER = "c".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:demo`;

function pullRequestEvent(overrides = {}) {
  return {
    id: "f".repeat(64),
    kind: 1618,
    pubkey: AUTHOR,
    created_at: 100,
    content: "Add feature\n\nDetails.",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Add feature"],
      ["c", "1111111111111111111111111111111111111111"],
      ["clone", `https://relay.example/git/${OWNER}/demo`],
    ],
    ...overrides,
  };
}

function updateEvent({ pubkey, createdAt, commit, cloneUrl }) {
  return {
    id: `update-${pubkey.slice(0, 8)}-${createdAt}`,
    kind: 1619,
    pubkey,
    created_at: createdAt,
    content: "",
    tags: [
      ["E", "f".repeat(64)],
      ["a", REPO_ADDRESS],
      ["c", commit],
      ...(cloneUrl ? [["clone", cloneUrl]] : []),
    ],
  };
}

function statusEvent({ kind, pubkey, createdAt }) {
  return {
    id: `status-${pubkey.slice(0, 8)}-${createdAt}`,
    kind,
    pubkey,
    created_at: createdAt,
    content: "",
    tags: [
      ["e", "f".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
    ],
  };
}

test("accepts updates signed by the PR author", () => {
  const update = updateEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    commit: "2222222222222222222222222222222222222222",
    cloneUrl: `https://relay.example/git/${AUTHOR}/demo-fork`,
  });

  const pullRequest = eventToProjectPullRequest(pullRequestEvent(), [update]);

  assert.equal(pullRequest.commit, "2222222222222222222222222222222222222222");
  assert.deepEqual(pullRequest.cloneUrls, [
    `https://relay.example/git/${AUTHOR}/demo-fork`,
  ]);
  assert.equal(pullRequest.updateCount, 1);
});

test("accepts updates signed by the repo owner", () => {
  const update = updateEvent({
    pubkey: OWNER,
    createdAt: 200,
    commit: "3333333333333333333333333333333333333333",
  });

  const pullRequest = eventToProjectPullRequest(pullRequestEvent(), [update]);

  assert.equal(pullRequest.commit, "3333333333333333333333333333333333333333");
});

test("ignores a later update from a different pubkey", () => {
  const authorUpdate = updateEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    commit: "2222222222222222222222222222222222222222",
    cloneUrl: `https://relay.example/git/${OWNER}/demo`,
  });
  const attackerUpdate = updateEvent({
    pubkey: ATTACKER,
    createdAt: 300,
    commit: "6666666666666666666666666666666666666666",
    cloneUrl: "https://evil.example/git/attacker/repo",
  });

  const pullRequest = eventToProjectPullRequest(pullRequestEvent(), [
    authorUpdate,
    attackerUpdate,
  ]);

  assert.equal(pullRequest.commit, "2222222222222222222222222222222222222222");
  assert.deepEqual(pullRequest.cloneUrls, [
    `https://relay.example/git/${OWNER}/demo`,
  ]);
  assert.equal(pullRequest.updateCount, 1);
});

test("ignores status events from a different pubkey", () => {
  const attackerMerged = statusEvent({
    kind: 1631,
    pubkey: ATTACKER,
    createdAt: 300,
  });

  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [attackerMerged],
  );

  assert.equal(pullRequest.status, "Open");
});

test("honors status events from the PR author and repo owner", () => {
  const authorMerged = statusEvent({
    kind: 1631,
    pubkey: AUTHOR,
    createdAt: 300,
  });
  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [authorMerged],
  );
  assert.equal(pullRequest.status, "Merged");

  const ownerClosed = statusEvent({
    kind: 1632,
    pubkey: OWNER,
    createdAt: 400,
  });
  const closedPullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [ownerClosed],
  );
  assert.equal(closedPullRequest.status, "Closed");
});

test("survives malformed value-less tags", () => {
  const event = pullRequestEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["t"],
      ["p"],
      ["c", "1111111111111111111111111111111111111111"],
    ],
  });

  const pullRequest = eventToProjectPullRequest(event);

  assert.equal(pullRequest.status, "Open");
  assert.deepEqual(pullRequest.labels, []);
  assert.deepEqual(pullRequest.recipients, []);
});
