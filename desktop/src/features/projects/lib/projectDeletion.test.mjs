import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isProjectDeletedByAddress,
  projectRepoAddress,
} from "./projectDeletion.ts";

const KIND = 30617;
const OWNER = "a".repeat(64);
const OTHER = "b".repeat(64);
const DTAG = "bimblepath";
const COORD = `${KIND}:${OWNER}:${DTAG}`;

function project(createdAt) {
  return { owner: OWNER, dtag: DTAG, createdAt };
}

function deletion({ pubkey = OWNER, createdAt, a = COORD }) {
  return {
    pubkey,
    created_at: createdAt,
    tags: [["a", a]],
  };
}

test("projectRepoAddress builds 30617:owner:dtag", () => {
  assert.equal(projectRepoAddress(KIND, { owner: OWNER, dtag: DTAG }), COORD);
});

test("no deletions → not deleted", () => {
  assert.equal(isProjectDeletedByAddress(KIND, project(100), []), false);
});

test("owner delete at or after announcement hides the project", () => {
  assert.equal(
    isProjectDeletedByAddress(KIND, project(100), [
      deletion({ createdAt: 100 }),
    ]),
    true,
  );
  assert.equal(
    isProjectDeletedByAddress(KIND, project(100), [
      deletion({ createdAt: 101 }),
    ]),
    true,
  );
});

test("older owner delete does not hide a newer re-announce (resurrect)", () => {
  // Real pyxe case: delete at T, re-create same d-tag at T+104.
  assert.equal(
    isProjectDeletedByAddress(KIND, project(204), [
      deletion({ createdAt: 100 }),
    ]),
    false,
  );
});

test("foreign pubkey delete cannot hide the project", () => {
  assert.equal(
    isProjectDeletedByAddress(KIND, project(100), [
      deletion({ pubkey: OTHER, createdAt: 200 }),
    ]),
    false,
  );
});

test("delete for a different address is ignored", () => {
  assert.equal(
    isProjectDeletedByAddress(KIND, project(100), [
      deletion({
        createdAt: 200,
        a: `${KIND}:${OWNER}:other-repo`,
      }),
    ]),
    false,
  );
});

test("owner case is compared case-insensitively", () => {
  assert.equal(
    isProjectDeletedByAddress(
      KIND,
      { owner: OWNER.toUpperCase(), dtag: DTAG, createdAt: 100 },
      [deletion({ pubkey: OWNER.toLowerCase(), createdAt: 100 })],
    ),
    true,
  );
});
