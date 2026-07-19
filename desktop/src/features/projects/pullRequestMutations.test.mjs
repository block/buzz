import assert from "node:assert/strict";
import test from "node:test";

import {
  canPublishProjectPullRequestUpdate,
  projectPullRequestMergedTags,
  projectPullRequestTags,
  projectPullRequestUpdateTags,
} from "./pullRequestMutations.ts";

const OWNER = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const REVIEWER = "c".repeat(64);
const PR_ID = "d".repeat(64);
const COMMIT = "e".repeat(40);
const MERGE_BASE = "f".repeat(40);

const project = {
  owner: OWNER,
  repoAddress: `30617:${OWNER}:buzz`,
  cloneUrls: [`https://relay.example/git/${OWNER}/buzz`],
};
const pullRequest = {
  author: AUTHOR,
};

test("only the repository owner or PR author can publish an update", () => {
  assert.equal(
    canPublishProjectPullRequestUpdate(OWNER, project, pullRequest),
    true,
  );
  assert.equal(
    canPublishProjectPullRequestUpdate(AUTHOR, project, pullRequest),
    true,
  );
  assert.equal(
    canPublishProjectPullRequestUpdate(REVIEWER, project, pullRequest),
    false,
  );
});

test("projectPullRequestTags builds a NIP-34 kind 1618 tag set", () => {
  const tags = projectPullRequestTags(project, {
    title: "Add Projects workflow",
    body: "",
    branch: "projects-workflow",
    targetBranch: "main",
    commit: COMMIT,
    mergeBase: MERGE_BASE,
    reviewers: [REVIEWER, REVIEWER.toUpperCase()],
  });

  assert.deepEqual(tags, [
    ["a", project.repoAddress],
    ["p", OWNER],
    ["p", REVIEWER],
    ["subject", "Add Projects workflow"],
    ["c", COMMIT],
    ["clone", project.cloneUrls[0]],
    ["branch-name", "projects-workflow"],
    ["target-branch", "main"],
    ["merge-base", MERGE_BASE],
  ]);
});

test("projectPullRequestUpdateTags uses uppercase NIP-22 root tags", () => {
  const forkCloneUrl = `https://relay.example/git/${AUTHOR}/buzz`;
  const tags = projectPullRequestUpdateTags(
    project,
    { id: PR_ID, author: AUTHOR, cloneUrls: [forkCloneUrl] },
    COMMIT,
    MERGE_BASE,
  );

  assert.ok(tags.some((tag) => tag[0] === "E" && tag[1] === PR_ID));
  assert.ok(tags.some((tag) => tag[0] === "P" && tag[1] === AUTHOR));
  assert.ok(tags.some((tag) => tag[0] === "c" && tag[1] === COMMIT));
  assert.ok(
    tags.some(
      (tag) =>
        tag[0] === "clone" &&
        tag[1] === forkCloneUrl &&
        !tag.includes(project.cloneUrls[0]),
    ),
  );
});

test("projectPullRequestMergedTags records the pushed merge commit", () => {
  const tags = projectPullRequestMergedTags(
    project,
    { id: PR_ID, author: AUTHOR },
    COMMIT,
  );

  assert.deepEqual(tags, [
    ["e", PR_ID, "", "root"],
    ["a", project.repoAddress],
    ["p", OWNER],
    ["p", AUTHOR],
    ["merge-commit", COMMIT],
    ["r", COMMIT],
  ]);
});
