import assert from "node:assert/strict";
import { test } from "node:test";

import {
  projectRepoUnavailableReason,
  refineRepoUnavailableReason,
} from "./projectRepoAvailability.ts";

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

test("refines a masked 404 into an unbound-repository reason", () => {
  assert.equal(
    refineRepoUnavailableReason({
      reason: "missing",
      repositoryChannelId: null,
      memberChannelIds: ["11111111-1111-4111-8111-111111111111"],
    }),
    "unbound",
  );
});

test("refines a masked 404 into an access-denied reason for non-members", () => {
  assert.equal(
    refineRepoUnavailableReason({
      reason: "missing",
      repositoryChannelId: "22222222-2222-4222-8222-222222222222",
      memberChannelIds: ["11111111-1111-4111-8111-111111111111"],
    }),
    "access",
  );
});

test("keeps missing when the viewer is a member of the bound channel", () => {
  assert.equal(
    refineRepoUnavailableReason({
      reason: "missing",
      repositoryChannelId: "11111111-1111-4111-8111-111111111111",
      memberChannelIds: ["11111111-1111-4111-8111-111111111111"],
    }),
    "missing",
  );
});

test("does not guess while memberships are still loading", () => {
  assert.equal(
    refineRepoUnavailableReason({
      reason: "missing",
      repositoryChannelId: "22222222-2222-4222-8222-222222222222",
      memberChannelIds: null,
    }),
    "missing",
  );
});

test("never rewrites non-missing reasons", () => {
  assert.equal(
    refineRepoUnavailableReason({
      reason: "network",
      repositoryChannelId: null,
      memberChannelIds: [],
    }),
    "network",
  );
});
