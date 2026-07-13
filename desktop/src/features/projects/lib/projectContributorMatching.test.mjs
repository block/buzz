import assert from "node:assert/strict";
import { test } from "node:test";

import {
  commitAuthorPubkeysFromPullRequests,
  profileForCommit,
} from "./projectContributorMatching.ts";

const AGENT_PUBKEY = "a".repeat(64);
const USER_PUBKEY = "b".repeat(64);

const PROFILES = {
  [AGENT_PUBKEY]: {
    displayName: "Brain",
    avatarUrl: "https://example.com/brain.png",
    isAgent: true,
  },
  [USER_PUBKEY]: {
    displayName: "Thomas P",
    avatarUrl: "https://example.com/thomas.png",
    nip05Handle: "thomasp@example.com",
  },
};

function makeCommit(overrides = {}) {
  return {
    hash: "fed2b2c993896352400f3d8c574fa31a84188f18",
    shortHash: "fed2b2c",
    authorName: "Thomas Petersen",
    authorEmail: "thomasp@squareup.com",
    timestamp: 1_700_000_000,
    subject: "Add simple score HUD",
    ...overrides,
  };
}

test("maps initial, latest, and update commits to their publishers", () => {
  const map = commitAuthorPubkeysFromPullRequests([
    {
      author: AGENT_PUBKEY,
      initialCommit: "AAAA000000000000000000000000000000000000",
      commit: "cccc000000000000000000000000000000000000",
      updates: [
        {
          author: USER_PUBKEY,
          commit: "cccc000000000000000000000000000000000000",
        },
      ],
    },
  ]);

  // Hashes are normalized to lowercase.
  assert.equal(
    map.get("aaaa000000000000000000000000000000000000"),
    AGENT_PUBKEY,
  );
  // Updates win over the PR-level latest commit: they carry the publisher
  // of that specific push.
  assert.equal(
    map.get("cccc000000000000000000000000000000000000"),
    USER_PUBKEY,
  );
});

test("profileForCommit prefers the signed PR-event mapping", () => {
  const commit = makeCommit();
  const map = new Map([[commit.hash, AGENT_PUBKEY]]);

  const matched = profileForCommit(commit, PROFILES, map);
  assert.equal(matched?.pubkey, AGENT_PUBKEY);
  assert.equal(matched?.profile.displayName, "Brain");
});

test("profileForCommit falls back to exact git author matching", () => {
  // No mapping entry — the git author email matches the user's NIP-05.
  const commit = makeCommit({ authorEmail: "thomasp@example.com" });
  const matched = profileForCommit(commit, PROFILES, new Map());
  assert.equal(matched?.pubkey, USER_PUBKEY);
});

test("profileForCommit returns null when nothing matches", () => {
  const commit = makeCommit();
  assert.equal(profileForCommit(commit, PROFILES, new Map()), null);
});

test("mapped pubkey without a fetched profile falls back to git author", () => {
  const commit = makeCommit({ authorEmail: "thomasp@example.com" });
  const map = new Map([[commit.hash, "f".repeat(64)]]);
  const matched = profileForCommit(commit, PROFILES, map);
  assert.equal(matched?.pubkey, USER_PUBKEY);
});

test("viewer git identity attributes their own commits", () => {
  // Git author "Thomas Petersen <thomasp@squareup.com>" matches no profile
  // field, but it is the viewer's own git config identity.
  const commit = makeCommit();
  const matched = profileForCommit(commit, PROFILES, new Map(), {
    pubkey: USER_PUBKEY,
    name: "Thomas Petersen",
    email: "thomasp@squareup.com",
  });
  assert.equal(matched?.pubkey, USER_PUBKEY);
  assert.equal(matched?.profile.displayName, "Thomas P");
});

test("viewer git identity does not claim other authors' commits", () => {
  const commit = makeCommit({
    authorName: "Someone Else",
    authorEmail: "someone@example.com",
  });
  const matched = profileForCommit(commit, PROFILES, new Map(), {
    pubkey: USER_PUBKEY,
    name: "Thomas Petersen",
    email: "thomasp@squareup.com",
  });
  assert.equal(matched, null);
});

test("signed PR mapping wins over the viewer git identity", () => {
  const commit = makeCommit();
  const map = new Map([[commit.hash, AGENT_PUBKEY]]);
  const matched = profileForCommit(commit, PROFILES, map, {
    pubkey: USER_PUBKEY,
    name: "Thomas Petersen",
    email: "thomasp@squareup.com",
  });
  assert.equal(matched?.pubkey, AGENT_PUBKEY);
});
