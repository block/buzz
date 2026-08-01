import assert from "node:assert/strict";
import test from "node:test";

import { isDeletedByA } from "./hooks.ts";

const OWNER = "a".repeat(64);
const ATTACKER = "c".repeat(64);

function makeProject(createdAt = 200) {
  return {
    id: `${OWNER}:demo`,
    dtag: "demo",
    name: "Demo",
    description: "",
    cloneUrls: [],
    webUrl: null,
    owner: OWNER,
    contributors: [],
    createdAt,
    projectChannelId: null,
    status: "active",
    defaultBranch: "main",
    repoAddress: `30617:${OWNER}:demo`,
  };
}

function makeDeletionEvent({
  pubkey = OWNER,
  createdAt = 300,
  coordinate = `30617:${OWNER}:demo`,
}) {
  return {
    id: "d".repeat(64),
    kind: 5,
    pubkey,
    created_at: createdAt,
    content: "",
    tags: [["a", coordinate]],
    sig: "f".repeat(128),
  };
}

test("deletion event newer than announcement hides project", () => {
  const project = makeProject(100);
  const deletion = makeDeletionEvent({ createdAt: 200 });
  assert.ok(isDeletedByA(project, [deletion]));
});

test("deletion event older than re-announcement does NOT hide project", () => {
  const project = makeProject(400);
  const deletion = makeDeletionEvent({ createdAt: 300 });
  assert.ok(!isDeletedByA(project, [deletion]));
});

test("deletion event at same timestamp as announcement does not hide", () => {
  const project = makeProject(200);
  const deletion = makeDeletionEvent({ createdAt: 200 });
  assert.ok(!isDeletedByA(project, [deletion]));
});

test("deletion from non-owner does not hide project", () => {
  const project = makeProject(100);
  const deletion = makeDeletionEvent({ pubkey: ATTACKER, createdAt: 300 });
  assert.ok(!isDeletedByA(project, [deletion]));
});

test("deletion targeting different coordinate does not hide this one", () => {
  const project = makeProject(100);
  const deletion = makeDeletionEvent({
    createdAt: 300,
    coordinate: `30617:${OWNER}:other-repo`,
  });
  assert.ok(!isDeletedByA(project, [deletion]));
});

test("mixed deletion events: only newer-than-announcement ones hide", () => {
  const project = makeProject(400);
  const staleDeletion = makeDeletionEvent({ createdAt: 300 });
  const freshDeletion = makeDeletionEvent({ createdAt: 500 });
  assert.ok(!isDeletedByA(project, [staleDeletion]));
  assert.ok(isDeletedByA(project, [staleDeletion, freshDeletion]));
});
