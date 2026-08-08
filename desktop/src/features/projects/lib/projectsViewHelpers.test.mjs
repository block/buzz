import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isProjectAccessibleToViewer,
  isRepositoryAccessibleToViewer,
  relativeTime,
} from "./projectsViewHelpers.ts";

const DAY_SECONDS = 24 * 60 * 60;

function localSeconds(year, month, day) {
  return Math.floor(new Date(year, month, day, 12).getTime() / 1_000);
}

const REPO_OWNER = "a".repeat(64);
const VIEWER = "b".repeat(64);
const BOUND_CHANNEL = "11111111-1111-4111-8111-111111111111";
const RELAY_ORIGIN = "https://relay.example";

function makeRepository(overrides = {}) {
  return {
    channelId: BOUND_CHANNEL,
    cloneUrls: [`${RELAY_ORIGIN}/git/${REPO_OWNER}/repo-a`],
    contributors: [],
    createdAt: 0,
    defaultBranch: "main",
    dtag: "repo-a",
    id: `${REPO_OWNER}:repo-a`,
    name: "repo-a",
    owner: REPO_OWNER,
    repoAddress: `30617:${REPO_OWNER}:repo-a`,
    ...overrides,
  };
}

function makeAccessInput(overrides = {}) {
  return {
    currentPubkey: VIEWER,
    localRepoNames: new Set(),
    memberChannelIds: [],
    relayOrigin: RELAY_ORIGIN,
    ...overrides,
  };
}

test("a channel-bound repository is accessible only to channel members", () => {
  const repository = makeRepository();

  assert.equal(
    isRepositoryAccessibleToViewer(
      repository,
      makeAccessInput({ memberChannelIds: [BOUND_CHANNEL] }),
    ),
    true,
  );
  assert.equal(
    isRepositoryAccessibleToViewer(
      repository,
      makeAccessInput({ memberChannelIds: [] }),
    ),
    false,
  );
});

test("an unbound repository stays accessible to its owner", () => {
  const repository = makeRepository({ channelId: null });

  assert.equal(
    isRepositoryAccessibleToViewer(repository, makeAccessInput()),
    false,
  );
  assert.equal(
    isRepositoryAccessibleToViewer(
      repository,
      makeAccessInput({ currentPubkey: REPO_OWNER }),
    ),
    true,
  );
});

test("external hosting and local checkouts bypass the channel gate", () => {
  assert.equal(
    isRepositoryAccessibleToViewer(
      makeRepository({ cloneUrls: ["https://github.com/acme/site.git"] }),
      makeAccessInput(),
    ),
    true,
  );
  assert.equal(
    isRepositoryAccessibleToViewer(
      makeRepository(),
      makeAccessInput({ localRepoNames: new Set(["repo-a"]) }),
    ),
    true,
  );
});

test("channel-bound repositories stay visible while memberships load", () => {
  assert.equal(
    isRepositoryAccessibleToViewer(
      makeRepository(),
      makeAccessInput({ memberChannelIds: null }),
    ),
    true,
  );
});

test("a project is accessible when any repository is, or when owned", () => {
  const accessible = makeRepository({ channelId: BOUND_CHANNEL });
  const restricted = makeRepository({
    channelId: "22222222-2222-4222-8222-222222222222",
    dtag: "repo-b",
    id: `${REPO_OWNER}:repo-b`,
    name: "repo-b",
    repoAddress: `30617:${REPO_OWNER}:repo-b`,
  });
  const input = makeAccessInput({ memberChannelIds: [BOUND_CHANNEL] });

  assert.equal(
    isProjectAccessibleToViewer(
      { owner: REPO_OWNER, repositories: [restricted, accessible] },
      input,
    ),
    true,
  );
  assert.equal(
    isProjectAccessibleToViewer(
      { owner: REPO_OWNER, repositories: [restricted] },
      input,
    ),
    false,
  );
  assert.equal(
    isProjectAccessibleToViewer(
      { owner: VIEWER, repositories: [restricted] },
      input,
    ),
    true,
  );
});

test("relativeTime switches to an absolute date at seven days", () => {
  const now = localSeconds(2025, 5, 15);

  assert.equal(relativeTime(now - 7 * DAY_SECONDS + 1, now), "6 days ago");

  const createdAt = now - 7 * DAY_SECONDS;
  const expected = new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  assert.equal(relativeTime(createdAt, now), expected);
});

test("relativeTime includes the year only across a year boundary", () => {
  const sameYearNow = localSeconds(2025, 5, 15);
  const sameYearCreatedAt = sameYearNow - 7 * DAY_SECONDS;
  const sameYearExpected = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(new Date(sameYearCreatedAt * 1_000));
  assert.equal(relativeTime(sameYearCreatedAt, sameYearNow), sameYearExpected);

  const crossYearNow = localSeconds(2025, 0, 8);
  const crossYearCreatedAt = localSeconds(2024, 11, 31);
  const crossYearExpected = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(crossYearCreatedAt * 1_000));
  assert.equal(
    relativeTime(crossYearCreatedAt, crossYearNow),
    crossYearExpected,
  );
});
