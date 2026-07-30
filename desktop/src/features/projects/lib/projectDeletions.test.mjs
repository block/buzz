import assert from "node:assert/strict";
import { test } from "node:test";

import { isDeletedByA, projectCoordinate } from "./projectDeletions.ts";

const OWNER = "a".repeat(64);
const OTHER = "b".repeat(64);

const ANNOUNCED_AT = 1_700_000_000;

function project(overrides = {}) {
  return {
    owner: OWNER,
    dtag: "bitchat",
    createdAt: ANNOUNCED_AT,
    ...overrides,
  };
}

function deletion({
  pubkey = OWNER,
  createdAt = ANNOUNCED_AT,
  coordinate,
} = {}) {
  return {
    id: "deadbeef",
    pubkey,
    kind: 5,
    created_at: createdAt,
    content: "",
    tags: [["a", coordinate ?? projectCoordinate(project())]],
    sig: "",
  };
}

test("a tombstone older than the announcement does not hide it", () => {
  // The #3760 regression: delete a repo, then announce the same dtag again.
  // The re-announcement is newer than the tombstone, so it is live.
  assert.equal(
    isDeletedByA(project(), [deletion({ createdAt: ANNOUNCED_AT - 1 })]),
    false,
  );
});

test("a tombstone newer than the announcement hides it", () => {
  assert.equal(
    isDeletedByA(project(), [deletion({ createdAt: ANNOUNCED_AT + 1 })]),
    true,
  );
});

test("a tombstone at the announcement's own timestamp hides it", () => {
  // NIP-09 deletes versions "up to the created_at timestamp" — inclusive.
  assert.equal(isDeletedByA(project(), [deletion()]), true);
});

test("only the announcement author can delete it", () => {
  assert.equal(
    isDeletedByA(project(), [
      deletion({ pubkey: OTHER, createdAt: ANNOUNCED_AT + 1 }),
    ]),
    false,
  );
});

test("author matching stays case-insensitive", () => {
  assert.equal(
    isDeletedByA(project(), [
      deletion({ pubkey: OWNER.toUpperCase(), createdAt: ANNOUNCED_AT + 1 }),
    ]),
    true,
  );
});

test("a tombstone for a different coordinate is ignored", () => {
  assert.equal(
    isDeletedByA(project(), [
      deletion({
        createdAt: ANNOUNCED_AT + 1,
        coordinate: projectCoordinate({ owner: OWNER, dtag: "other-repo" }),
      }),
    ]),
    false,
  );
});

test("a newer tombstone still hides the announcement when an older one exists", () => {
  assert.equal(
    isDeletedByA(project(), [
      deletion({ createdAt: ANNOUNCED_AT - 100 }),
      deletion({ createdAt: ANNOUNCED_AT + 100 }),
    ]),
    true,
  );
});

test("projectCoordinate builds the NIP-34 repo address", () => {
  assert.equal(
    projectCoordinate({ owner: OWNER, dtag: "bitchat" }),
    `30617:${OWNER}:bitchat`,
  );
});
