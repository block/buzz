import assert from "node:assert/strict";
import { test } from "node:test";

import { isProjectHiddenByDeletion } from "./lib/projectDeletionFilter.ts";

const OWNER = "aa".repeat(32);
const COORDINATE = `30617:${OWNER}:bitchat`;

function deletionEvent(createdAt) {
  return {
    id: `del-${createdAt}`,
    pubkey: OWNER,
    created_at: createdAt,
    kind: 5,
    tags: [["a", COORDINATE]],
    content: "",
    sig: "",
  };
}

test("isProjectHiddenByDeletion ignores deletions older than the announcement", () => {
  const project = {
    owner: OWNER,
    dtag: "bitchat",
    createdAt: 2_000,
  };

  assert.equal(
    isProjectHiddenByDeletion(project, [deletionEvent(1_000)]),
    false,
    "stale deletion must not hide a newer re-announcement",
  );
  assert.equal(
    isProjectHiddenByDeletion(project, [deletionEvent(2_000)]),
    true,
    "deletion at the same second applies",
  );
  assert.equal(
    isProjectHiddenByDeletion(project, [deletionEvent(3_000)]),
    true,
    "deletion after the announcement applies",
  );
});

test("isProjectHiddenByDeletion ignores deletions from other authors", () => {
  const project = {
    owner: OWNER,
    dtag: "bitchat",
    createdAt: 100,
  };
  const foreignDeletion = {
    ...deletionEvent(200),
    pubkey: "bb".repeat(32),
  };

  assert.equal(isProjectHiddenByDeletion(project, [foreignDeletion]), false);
});
