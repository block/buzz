import assert from "node:assert/strict";
import test from "node:test";

import { canHealProjectHomeRepositories } from "./useHealProjectHomeRepositories.ts";

const OWNER = "a".repeat(64);
const PROJECT = {
  owner: OWNER,
  repositories: [{ id: "repository" }],
};

test("snapshot-derived projects cannot trigger repository healing", () => {
  assert.equal(
    canHealProjectHomeRepositories({
      identityPubkey: OWNER,
      project: PROJECT,
      projectDataIsAuthoritative: false,
    }),
    false,
  );
});

test("authoritative owner projects with repositories can heal", () => {
  assert.equal(
    canHealProjectHomeRepositories({
      identityPubkey: OWNER,
      project: PROJECT,
      projectDataIsAuthoritative: true,
    }),
    true,
  );
});

test("repository healing still rejects a non-owner identity", () => {
  assert.equal(
    canHealProjectHomeRepositories({
      identityPubkey: "b".repeat(64),
      project: PROJECT,
      projectDataIsAuthoritative: true,
    }),
    false,
  );
});
