import assert from "node:assert/strict";
import { test } from "node:test";

import {
  commitAuthorDisplayName,
  commitAuthorPubkeysFromPullRequests,
  gitContributorPubkeysFromCommits,
  gitAuthorFieldDisplayValue,
  profileForCommit,
  profileForContributor,
  projectContributorActivityCounts,
  signedProjectContributorPubkeys,
} from "./projectContributorMatching.ts";
import { pubkeyToNpub } from "../../../shared/lib/nostrUtils.ts";

const AGENT_PUBKEY =
  "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const USER_PUBKEY =
  "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
const REVIEWER_PUBKEY =
  "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
const COMMENTER_ONE_PUBKEY =
  "e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13";
const COMMENTER_TWO_PUBKEY =
  "2f8bde4d1a07209355b4a7250a5c5128e88b84bddc619ab7cba8d569b240efe4";
const MISSING_PROFILE_PUBKEY =
  "fff97bd5755eeea420453a14355235d382f6472f8568a18b2f057a1460297556";
const USER_NPUB = pubkeyToNpub(USER_PUBKEY);

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

test("collects declared humans and signed agent activity without assignments", () => {
  const reviewer = REVIEWER_PUBKEY;
  const commenters = [COMMENTER_ONE_PUBKEY, COMMENTER_TWO_PUBKEY];
  const contributors = signedProjectContributorPubkeys({
    owner: USER_PUBKEY,
    contributors: [AGENT_PUBKEY],
    pullRequests: [
      {
        author: AGENT_PUBKEY,
        approvals: [{ author: reviewer }],
        comments: [{ author: commenters[0] }],
        updates: [{ author: USER_PUBKEY, commit: null }],
      },
    ],
    issues: [
      {
        author: USER_PUBKEY,
        comments: [{ author: commenters[1] }],
      },
    ],
  });

  assert.deepEqual(contributors, [
    USER_PUBKEY,
    AGENT_PUBKEY,
    commenters[0],
    reviewer,
    commenters[1],
  ]);
});

test("counts signed commits, reviews, and tasks by contributor", () => {
  const counts = projectContributorActivityCounts({
    owner: USER_PUBKEY,
    contributors: [AGENT_PUBKEY],
    pullRequests: [
      {
        author: AGENT_PUBKEY,
        initialCommit: "1".repeat(40),
        commit: "2".repeat(40),
        approvals: [],
        comments: [],
        updates: [{ author: USER_PUBKEY, commit: "2".repeat(40) }],
      },
    ],
    issues: [
      {
        author: USER_PUBKEY,
        comments: [],
      },
    ],
  });

  assert.deepEqual(counts[AGENT_PUBKEY], {
    commits: 1,
    reviews: 1,
    tasks: 0,
  });
  assert.deepEqual(counts[USER_PUBKEY], {
    commits: 1,
    reviews: 0,
    tasks: 1,
  });
});

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

test("links Git contributors through commit hashes from signed reviews", () => {
  const commit = makeCommit({ authorEmail: "wes@example.com" });
  const linked = gitContributorPubkeysFromCommits(
    [commit],
    [
      {
        author: USER_PUBKEY,
        initialCommit: commit.hash,
        commit: commit.hash,
        updates: [],
      },
    ],
  );

  assert.equal(linked.get("wes@example.com"), USER_PUBKEY);
});

test("does not link a Git identity claimed by multiple signed publishers", () => {
  const first = makeCommit({
    hash: "1".repeat(40),
    shortHash: "1".repeat(7),
    authorEmail: "shared@example.com",
  });
  const second = makeCommit({
    hash: "2".repeat(40),
    shortHash: "2".repeat(7),
    authorEmail: "shared@example.com",
  });
  const linked = gitContributorPubkeysFromCommits(
    [first, second],
    [
      {
        author: USER_PUBKEY,
        initialCommit: first.hash,
        commit: first.hash,
        updates: [],
      },
      {
        author: AGENT_PUBKEY,
        initialCommit: second.hash,
        commit: second.hash,
        updates: [],
      },
    ],
  );

  assert.equal(linked.has("shared@example.com"), false);
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

test("profileForCommit resolves an npub git author to its profile", () => {
  const commit = makeCommit({ authorName: USER_NPUB });
  const matched = profileForCommit(commit, PROFILES, new Map());
  assert.equal(matched?.pubkey, USER_PUBKEY);
  assert.equal(matched?.profile.displayName, "Thomas P");
});

test("profileForContributor resolves canonical npub git emails", () => {
  const matched = profileForContributor(
    {
      name: "Ordinary Git Name",
      email: USER_NPUB,
      commitCount: 1,
      lastCommitAt: 1_700_000_000,
    },
    PROFILES,
  );
  assert.equal(matched?.pubkey, USER_PUBKEY);
  assert.equal(matched?.profile.displayName, "Thomas P");
});

test("commitAuthorDisplayName canonicalizes keys and preserves ordinary Git text", () => {
  assert.equal(
    commitAuthorDisplayName(
      makeCommit({ authorName: USER_PUBKEY, authorEmail: "" }),
    ),
    USER_NPUB,
  );
  assert.equal(
    commitAuthorDisplayName(
      makeCommit({ authorName: USER_NPUB, authorEmail: "" }),
    ),
    USER_NPUB,
  );
  assert.equal(
    commitAuthorDisplayName(
      makeCommit({ authorName: "Ordinary Git Name", authorEmail: USER_PUBKEY }),
    ),
    USER_NPUB,
  );
  assert.equal(commitAuthorDisplayName(makeCommit()), "Thomas Petersen");
  assert.equal(
    commitAuthorDisplayName(
      makeCommit({ authorName: "", authorEmail: "thomas@example.com" }),
    ),
    "thomas@example.com",
  );
  assert.equal(
    commitAuthorDisplayName(makeCommit({ authorName: "", authorEmail: "" })),
    "Unknown author",
  );
});

test("gitAuthorFieldDisplayValue canonicalizes identity-shaped secondary text", () => {
  assert.equal(gitAuthorFieldDisplayValue(USER_PUBKEY), USER_NPUB);
  assert.equal(gitAuthorFieldDisplayValue(USER_NPUB), USER_NPUB);
  assert.equal(
    gitAuthorFieldDisplayValue(" thomas@example.com "),
    "thomas@example.com",
  );
});

test("profileForCommit ignores malformed npub git authors", () => {
  const commit = makeCommit({
    authorName: "npub1not-a-valid-key",
    authorEmail: "unknown@example.com",
  });
  assert.equal(profileForCommit(commit, PROFILES, new Map()), null);
});

test("profileForCommit returns null when nothing matches", () => {
  const commit = makeCommit();
  assert.equal(profileForCommit(commit, PROFILES, new Map()), null);
});

test("mapped pubkey without a fetched profile falls back to git author", () => {
  const commit = makeCommit({ authorEmail: "thomasp@example.com" });
  const map = new Map([[commit.hash, MISSING_PROFILE_PUBKEY]]);
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

test("a shared display name alone never borrows the viewer's identity", () => {
  // Two contributors can share a display name; only the git email — which
  // the viewer's own commits actually carry — may attribute a commit to the
  // viewer's pubkey.
  const commit = makeCommit({
    authorName: "Thomas Petersen",
    authorEmail: "impostor@example.org",
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
