import assert from "node:assert/strict";
import { test } from "node:test";

import { projectGitDataState } from "./projectGitDataState.ts";

test("github remotes with a snapshot are available, not blocked", () => {
  assert.equal(
    projectGitDataState({
      error: null,
      fileCount: 3,
      hasSnapshot: true,
      loading: false,
    }),
    "available",
  );
});

test("loading wins over a missing snapshot", () => {
  assert.equal(
    projectGitDataState({
      error: null,
      fileCount: 0,
      hasSnapshot: false,
      loading: true,
    }),
    "checking",
  );
});

test("empty trees stay empty", () => {
  assert.equal(
    projectGitDataState({
      error: null,
      fileCount: 0,
      hasSnapshot: true,
      loading: false,
    }),
    "empty",
  );
});

test("errors and missing snapshots stay unavailable", () => {
  assert.equal(
    projectGitDataState({
      error: new Error("401"),
      fileCount: 0,
      hasSnapshot: false,
      loading: false,
    }),
    "unavailable",
  );
});
