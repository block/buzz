import assert from "node:assert/strict";
import test from "node:test";

import { validateProjectDetailSearch } from "./projectDetailSearch.ts";

const COMMIT = "a".repeat(40);

test("validates existing search and repository target atomically", () => {
  assert.deepEqual(
    validateProjectDetailSearch({
      commitHash: "commit",
      pullRequestId: "pr",
      issueId: "issue",
      repositoryId: "repository",
      repoRef: COMMIT.toUpperCase(),
      repoPath: "src/main.ts",
    }),
    {
      commitHash: "commit",
      pullRequestId: "pr",
      issueId: "issue",
      repositoryId: "repository",
      repoRef: COMMIT,
      repoPath: "src/main.ts",
    },
  );
});

test("drops incomplete or unsafe repository target search without dropping repository selection", () => {
  assert.deepEqual(
    validateProjectDetailSearch({
      repositoryId: "repository",
      repoRef: "main",
    }),
    {
      commitHash: undefined,
      pullRequestId: undefined,
      issueId: undefined,
      repositoryId: "repository",
      repoRef: undefined,
      repoPath: undefined,
    },
  );
  assert.deepEqual(
    validateProjectDetailSearch({
      repositoryId: "repository",
      repoRef: "main",
      repoPath: "../secret",
    }),
    {
      commitHash: undefined,
      pullRequestId: undefined,
      issueId: undefined,
      repositoryId: "repository",
      repoRef: undefined,
      repoPath: undefined,
    },
  );
});
