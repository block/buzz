import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRepoLink,
  isRepoLink,
  isSafeRepoPath,
  parseRepoLink,
} from "./repoLink.ts";

const OWNER = "a".repeat(64);
const COMMIT = "b".repeat(40);

test("buildRepoLink produces a pinned owner-qualified link", () => {
  assert.equal(
    buildRepoLink({
      repoId: "boosh",
      owner: OWNER,
      ref: COMMIT,
      path: "reports/a b.html",
    }),
    `buzz://repo?repo=boosh&owner=${OWNER}&ref=${COMMIT}&path=reports%2Fa+b.html`,
  );
});

test("parseRepoLink returns the exact project, commit, and path", () => {
  const result = parseRepoLink(
    `buzz://repo?repo=boosh&owner=${OWNER}&ref=${COMMIT}&path=reports%2Fweekly.html`,
  );
  assert.deepEqual(result, {
    ok: true,
    value: {
      projectId: `${OWNER}:boosh`,
      repoId: "boosh",
      owner: OWNER,
      ref: COMMIT,
      path: "reports/weekly.html",
    },
  });
});

test("parseRepoLink accepts a legacy ownerless link", () => {
  const result = parseRepoLink(
    `buzz://repo?repo=boosh&ref=${COMMIT}&path=README.md`,
  );
  assert.equal(result.ok && result.value.projectId, "boosh");
});

test("repo links reject traversal, absolute paths, and abbreviated refs", () => {
  for (const path of [
    "../secret",
    "/etc/passwd",
    "reports//x.html",
    "reports\\x.html",
  ]) {
    assert.equal(isSafeRepoPath(path), false);
  }
  assert.equal(
    parseRepoLink(`buzz://repo?repo=boosh&ref=abc123&path=README.md`).ok,
    false,
  );
});

test("isRepoLink only recognizes the repo deep-link host", () => {
  assert.equal(isRepoLink("buzz://repo?repo=boosh"), true);
  assert.equal(isRepoLink("buzz://message?channel=x&id=y"), false);
});
