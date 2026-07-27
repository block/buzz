import assert from "node:assert/strict";
import test from "node:test";

import {
  parseProjectPullRequestMergeError,
  ProjectPullRequestMergeError,
} from "./projectGit.ts";

const CONFLICT = {
  code: "merge_conflict",
  message: "Pull request has merge conflicts.",
  recovery: {
    action: "open_terminal",
    targetBranch: "main",
    sourceBranch: "feature/demo",
  },
};

test("parses structured merge conflict recovery metadata", () => {
  const error = parseProjectPullRequestMergeError(CONFLICT);

  assert.ok(error instanceof ProjectPullRequestMergeError);
  assert.equal(error.code, "merge_conflict");
  assert.deepEqual(error.recovery, CONFLICT.recovery);
});

test("parses JSON-serialized Tauri merge errors", () => {
  const error = parseProjectPullRequestMergeError(JSON.stringify(CONFLICT));

  assert.ok(error instanceof ProjectPullRequestMergeError);
  assert.equal(error.message, "Pull request has merge conflicts.");
});

test("rejects malformed recovery metadata", () => {
  assert.equal(
    parseProjectPullRequestMergeError({
      ...CONFLICT,
      recovery: { ...CONFLICT.recovery, targetBranch: null },
    }),
    null,
  );
  assert.equal(parseProjectPullRequestMergeError(new Error("offline")), null);
});
