import assert from "node:assert/strict";
import { test } from "node:test";

import { projectRepoUnavailableReason } from "./projectRepoAvailability.ts";

test("classifies a missing repository", () => {
  assert.equal(
    projectRepoUnavailableReason(new Error("remote: Repository not found")),
    "missing",
  );
  assert.equal(projectRepoUnavailableReason(null), "missing");
});

test("classifies authentication failures before generic availability errors", () => {
  assert.equal(
    projectRepoUnavailableReason(
      new Error("The requested URL returned error: 403"),
    ),
    "authentication",
  );
  assert.equal(
    projectRepoUnavailableReason(new Error("Authentication failed")),
    "authentication",
  );
});

test("classifies branch and network failures", () => {
  assert.equal(
    projectRepoUnavailableReason(
      new Error("Remote branch main not found in upstream origin"),
    ),
    "ref",
  );
  assert.equal(
    projectRepoUnavailableReason(new Error("Could not resolve host: relay")),
    "network",
  );
  assert.equal(
    projectRepoUnavailableReason(new Error("git timed out after 300s")),
    "network",
  );
});

test("keeps unmatched failures generic", () => {
  assert.equal(
    projectRepoUnavailableReason(new Error("git exited with status 128")),
    "unknown",
  );
});
