import assert from "node:assert/strict";
import test from "node:test";

import {
  projectRepoSnapshotCloneUrl,
  projectRepoSnapshotTarget,
} from "./projectRepoSnapshotTarget.ts";

const COMMIT = "a".repeat(40);

test("explicit branch target outranks selected branch, tag, and pull request", () => {
  assert.deepEqual(
    projectRepoSnapshotTarget({
      selectedBranch: "main",
      projectDefaultBranch: "main",
      pullRequest: { id: "pr-1", commit: "b".repeat(40) },
      tag: { name: "v1", commit: "c".repeat(40) },
      repositoryRef: "feature/repo-links",
    }),
    {
      defaultBranch: "feature/repo-links",
      baseBranch: "main",
      targetRef: "refs/heads/feature/repo-links",
      targetCommit: null,
    },
  );
});

test("explicit commit target outranks branch, tag, and pull request", () => {
  assert.deepEqual(
    projectRepoSnapshotTarget({
      selectedBranch: "main",
      projectDefaultBranch: "main",
      pullRequest: { id: "pr-1", commit: "b".repeat(40) },
      tag: { name: "v1", commit: "c".repeat(40) },
      repositoryRef: COMMIT,
    }),
    {
      defaultBranch: "main",
      baseBranch: "main",
      targetRef: null,
      targetCommit: COMMIT,
    },
  );
});

test("existing tag and pull-request precedence is preserved without a repository link", () => {
  assert.deepEqual(
    projectRepoSnapshotTarget({
      selectedBranch: "release",
      projectDefaultBranch: "main",
      pullRequest: { id: "pr-1", commit: "b".repeat(40) },
      tag: { name: "v1", commit: "c".repeat(40) },
      repositoryRef: null,
    }),
    {
      defaultBranch: "release",
      baseBranch: "main",
      targetRef: "refs/tags/v1",
      targetCommit: "c".repeat(40),
    },
  );
});

test("explicit repository targets use the canonical project clone URL", () => {
  assert.equal(
    projectRepoSnapshotCloneUrl({
      projectCloneUrls: ["https://relay.example/git/owner/project"],
      pullRequestCloneUrls: ["https://fork.example/git/owner/fork"],
      repositoryTarget: { ref: "main", path: "README.md" },
    }),
    "https://relay.example/git/owner/project",
  );
});

test("ordinary pull request snapshots retain pull request clone precedence", () => {
  assert.equal(
    projectRepoSnapshotCloneUrl({
      projectCloneUrls: ["https://relay.example/git/owner/project"],
      pullRequestCloneUrls: ["https://fork.example/git/owner/fork"],
      repositoryTarget: null,
    }),
    "https://fork.example/git/owner/fork",
  );
});
