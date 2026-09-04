import assert from "node:assert/strict";
import test from "node:test";

import {
  currentPullRequestForSelection,
  projectReviewFilesChangedBody,
  retainLatestByKey,
  reviewDiffWorkspaceBranch,
  shouldReplaceRetainedPullRequest,
} from "./projectReviewDisplay.ts";

test("retainLatestByKey keeps the previous value when shouldReplace is false", () => {
  const cache = { current: { key: "pr-1", value: { files: [1] } } };

  const retained = retainLatestByKey(
    cache,
    "pr-1",
    { files: [] },
    (next, previous) =>
      next.files.length > 0 ? true : previous.files.length === 0,
  );

  assert.deepEqual(retained, { files: [1] });
  assert.deepEqual(cache.current.value, { files: [1] });
});

test("retainLatestByKey takes a new key immediately", () => {
  const cache = { current: { key: "pr-1", value: { files: [1] } } };

  const retained = retainLatestByKey(
    cache,
    "pr-2",
    { files: [] },
    (next) => next.files.length > 0,
  );

  assert.deepEqual(retained, { files: [] });
});

test("an explicit selected review does not fall back to another identity", () => {
  const selected = { id: "pr-a" };
  const branchReview = { id: "pr-branch" };

  assert.equal(
    currentPullRequestForSelection({
      fallback: branchReview,
      pullRequests: [selected, branchReview],
      selectedPullRequestId: "pr-a",
    }),
    selected,
  );
  assert.equal(
    currentPullRequestForSelection({
      fallback: branchReview,
      pullRequests: [branchReview],
      selectedPullRequestId: "pr-a",
    }),
    null,
  );
  assert.equal(
    currentPullRequestForSelection({
      fallback: branchReview,
      pullRequests: [branchReview],
      selectedPullRequestId: null,
    }),
    branchReview,
  );
});

test("retained review identity stays aligned with the diff-query identity across fetch phases", () => {
  const reviewA = { id: "pr-a" };
  const renderedCache = { current: { key: "repo:pr-a", value: reviewA } };
  const diffQueryCache = { current: { key: "repo:pr-a", value: reviewA } };

  const renderedDuringFetch = retainLatestByKey(
    renderedCache,
    "repo:pr-a",
    currentPullRequestForSelection({
      fallback: { id: "pr-branch" },
      pullRequests: [],
      selectedPullRequestId: "pr-a",
    }),
    (next) => shouldReplaceRetainedPullRequest(next, true),
  );
  const diffDuringFetch = retainLatestByKey(
    diffQueryCache,
    "repo:pr-a",
    currentPullRequestForSelection({
      fallback: { id: "pr-branch" },
      pullRequests: [],
      selectedPullRequestId: "pr-a",
    }),
    (next) => shouldReplaceRetainedPullRequest(next, true),
  );
  assert.equal(renderedDuringFetch, reviewA);
  assert.equal(diffDuringFetch, reviewA);
  assert.equal(renderedDuringFetch.id, diffDuringFetch.id);

  const renderedAfterComplete = retainLatestByKey(
    renderedCache,
    "repo:pr-a",
    currentPullRequestForSelection({
      fallback: { id: "pr-branch" },
      pullRequests: [],
      selectedPullRequestId: "pr-a",
    }),
    (next) => shouldReplaceRetainedPullRequest(next, false),
  );
  const diffAfterComplete = retainLatestByKey(
    diffQueryCache,
    "repo:pr-a",
    currentPullRequestForSelection({
      fallback: { id: "pr-branch" },
      pullRequests: [],
      selectedPullRequestId: "pr-a",
    }),
    (next) => shouldReplaceRetainedPullRequest(next, false),
  );
  assert.equal(renderedAfterComplete, null);
  assert.equal(diffAfterComplete, null);
});

test("review files stay mounted when a populated diff races an unavailable snapshot", () => {
  assert.equal(
    projectReviewFilesChangedBody({
      hasPopulatedDiff: true,
      hasSelectedPullRequest: true,
      repositoryUnavailable: true,
    }),
    "files",
  );
});

test("review files can show unavailable before a diff exists", () => {
  assert.equal(
    projectReviewFilesChangedBody({
      hasPopulatedDiff: false,
      hasSelectedPullRequest: true,
      repositoryUnavailable: true,
    }),
    "unavailable",
  );
});

test("review files render the panel for a selected review when the repo is available", () => {
  assert.equal(
    projectReviewFilesChangedBody({
      hasPopulatedDiff: false,
      hasSelectedPullRequest: true,
      repositoryUnavailable: false,
    }),
    "files",
  );
});

test("review diffs stay on the target branch, not the head or picker branch", () => {
  assert.equal(
    reviewDiffWorkspaceBranch({
      activeBranch: "variation/bees",
      defaultBranch: "main",
      pullRequest: { targetBranch: "main" },
    }),
    "main",
  );
  assert.equal(
    reviewDiffWorkspaceBranch({
      activeBranch: "variation/bees",
      defaultBranch: "main",
      pullRequest: { targetBranch: null },
    }),
    "main",
  );
  assert.equal(
    reviewDiffWorkspaceBranch({
      activeBranch: "variation/bees",
      defaultBranch: "main",
      pullRequest: null,
    }),
    "variation/bees",
  );
});
