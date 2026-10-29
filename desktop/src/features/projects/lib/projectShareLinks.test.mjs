import assert from "node:assert/strict";
import { test } from "node:test";

import {
  issueShareLink,
  parseAddressableCoordinate,
  projectShareLink,
  pullRequestShareLink,
  repositoryShareLink,
  shareTabForWorkspaceTab,
  workspaceTabForShareTab,
} from "./projectShareLinks.ts";

const OWNER = "a".repeat(64);
const EVENT_ID = "b".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:flappy-bee`;
const PROJECT_ADDRESS = `30621:${OWNER}:pollinator`;

test("parseAddressableCoordinate splits only the two structural separators", () => {
  assert.deepEqual(parseAddressableCoordinate(`30617:${OWNER}:a:b`), {
    kind: 30617,
    owner: OWNER,
    dtag: "a:b",
  });
  assert.deepEqual(
    parseAddressableCoordinate(`30617:${OWNER.toUpperCase()}:repo`)?.owner,
    OWNER,
  );
});

test("parseAddressableCoordinate rejects malformed coordinates", () => {
  for (const address of [
    null,
    undefined,
    "",
    OWNER,
    `30617:${OWNER}`,
    `30617:not-a-pubkey:repo`,
    `30617:${OWNER}:`,
    `:${OWNER}:repo`,
    `notakind:${OWNER}:repo`,
  ]) {
    assert.equal(parseAddressableCoordinate(address), null, String(address));
  }
});

test("projectShareLink links explicit projects by their 30621 coordinate", () => {
  assert.equal(
    projectShareLink({ projectAddress: PROJECT_ADDRESS }),
    `buzz://project?owner=${OWNER}&d=pollinator`,
  );
});

test("projectShareLink carries the active workspace tab for both link kinds", () => {
  assert.equal(
    projectShareLink({ projectAddress: PROJECT_ADDRESS }, "prs"),
    `buzz://project?owner=${OWNER}&d=pollinator&tab=prs`,
  );
  // Legacy projects share as buzz://repo and keep the tab too.
  assert.equal(
    projectShareLink({ projectAddress: REPO_ADDRESS }, "issues"),
    `buzz://repo?owner=${OWNER}&d=flappy-bee&tab=issues`,
  );
});

test("workspace tab ids map onto link tabs and back", () => {
  assert.equal(shareTabForWorkspaceTab("prs"), "prs");
  assert.equal(shareTabForWorkspaceTab("issues"), "issues");
  assert.equal(shareTabForWorkspaceTab("files"), "files");
  assert.equal(shareTabForWorkspaceTab("contributors"), "contributors");
  // "activity" is the workspace's name for the commit list.
  assert.equal(shareTabForWorkspaceTab("activity"), "commits");
  assert.equal(workspaceTabForShareTab("commits"), "activity");
  assert.equal(workspaceTabForShareTab("prs"), "prs");
  // Overview and PR-detail sub-tabs have no link spelling.
  assert.equal(shareTabForWorkspaceTab("overview"), undefined);
  assert.equal(shareTabForWorkspaceTab("pr-conversation"), undefined);
});

test("projectShareLink links legacy projects as their backing repository", () => {
  assert.equal(
    projectShareLink({ projectAddress: REPO_ADDRESS }),
    `buzz://repo?owner=${OWNER}&d=flappy-bee`,
  );
});

test("projectShareLink declines coordinates the link format cannot express", () => {
  for (const dtag of [
    "has space",
    "..",
    ".hidden",
    "x".repeat(65),
    "emoji🐝",
  ]) {
    assert.equal(
      projectShareLink({ projectAddress: `30621:${OWNER}:${dtag}` }),
      null,
      dtag,
    );
  }
  // Some other addressable kind is not a project or repository.
  assert.equal(projectShareLink({ projectAddress: `30000:${OWNER}:x` }), null);
});

test("repositoryShareLink links the repository coordinate", () => {
  assert.equal(
    repositoryShareLink({ repoAddress: REPO_ADDRESS }),
    `buzz://repo?owner=${OWNER}&d=flappy-bee`,
  );
  assert.equal(
    repositoryShareLink({ repoAddress: PROJECT_ADDRESS }),
    null,
    "a project coordinate is not a repository",
  );
});

test("issue and pull request links carry the event id and repo coordinate", () => {
  assert.equal(
    issueShareLink({ id: EVENT_ID, repoAddress: REPO_ADDRESS }),
    `buzz://issue?id=${EVENT_ID}&owner=${OWNER}&d=flappy-bee`,
  );
  assert.equal(
    pullRequestShareLink({ id: EVENT_ID, repoAddress: REPO_ADDRESS }),
    `buzz://pr?id=${EVENT_ID}&owner=${OWNER}&d=flappy-bee`,
  );
});

test("issue and pull request links require a repo coordinate and hex id", () => {
  assert.equal(issueShareLink({ id: EVENT_ID, repoAddress: null }), null);
  assert.equal(pullRequestShareLink({ id: EVENT_ID, repoAddress: null }), null);
  assert.equal(
    issueShareLink({ id: "short", repoAddress: REPO_ADDRESS }),
    null,
  );
  assert.equal(
    pullRequestShareLink({ id: "short", repoAddress: REPO_ADDRESS }),
    null,
  );
});
