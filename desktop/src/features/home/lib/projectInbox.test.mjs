import assert from "node:assert/strict";
import test from "node:test";

import { buildInboxItems, getInboxTypeLabel } from "./inbox.ts";
import { matchesInboxFilter } from "./inboxViewHelpers.ts";
import {
  getProjectInboxReference,
  isProjectInboxItem,
  resolveProjectInboxWorkItem,
} from "./projectInbox.ts";

const OWNER = "a".repeat(64);
const REVIEWER = "b".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:buzz`;
const PR_ID = "c".repeat(64);
const ISSUE_ID = "d".repeat(64);

function feedItem(overrides = {}) {
  return {
    id: PR_ID,
    kind: 1618,
    pubkey: OWNER,
    content: "Inbox support",
    createdAt: 1_700_000_000,
    channelId: null,
    channelName: "",
    tags: [
      ["a", REPO_ADDRESS],
      ["p", REVIEWER],
      ["subject", "Add project work items to Inbox"],
    ],
    category: "mention",
    ...overrides,
  };
}

const project = {
  id: "buzz",
  name: "Buzz",
  owner: OWNER,
  repoAddress: REPO_ADDRESS,
};

const pullRequest = {
  id: PR_ID,
  author: OWNER,
  title: "Add project work items to Inbox",
};

const issue = {
  id: ISSUE_ID,
  author: REVIEWER,
  title: "Inbox issue",
};

test("recognizes project roots and project thread activity", () => {
  assert.equal(isProjectInboxItem(feedItem()), true);
  assert.equal(
    isProjectInboxItem(
      feedItem({
        id: "e".repeat(64),
        kind: 1,
        tags: [
          ["a", REPO_ADDRESS],
          ["e", PR_ID, "", "root"],
          ["p", REVIEWER],
        ],
      }),
    ),
    true,
  );
  assert.equal(
    isProjectInboxItem(
      feedItem({
        kind: 9,
        tags: [
          ["a", REPO_ADDRESS],
          ["e", PR_ID, "", "root"],
        ],
      }),
    ),
    false,
  );
});

test("resolves the canonical project root from status and comment events", () => {
  assert.deepEqual(getProjectInboxReference(feedItem()), {
    repoAddress: REPO_ADDRESS,
    rootId: PR_ID,
  });
  assert.deepEqual(
    getProjectInboxReference(
      feedItem({
        id: "f".repeat(64),
        kind: 1631,
        tags: [
          ["a", REPO_ADDRESS],
          ["e", PR_ID, "", "root"],
        ],
      }),
    ),
    {
      repoAddress: REPO_ADDRESS,
      rootId: PR_ID,
    },
  );
});

test("matches a selected inbox event to its canonical pull request or issue", () => {
  const workItems = {
    pullRequests: {
      items: [{ project, pullRequest }],
      failedSections: [],
    },
    issues: {
      items: [{ project, issue }],
      failedSections: [],
    },
  };

  assert.deepEqual(resolveProjectInboxWorkItem(feedItem(), workItems), {
    type: "pull-request",
    project,
    pullRequest,
  });
  assert.deepEqual(
    resolveProjectInboxWorkItem(
      feedItem({
        id: ISSUE_ID,
        kind: 1621,
        tags: [
          ["a", REPO_ADDRESS],
          ["p", OWNER],
          ["subject", "Inbox issue"],
        ],
      }),
      workItems,
    ),
    {
      type: "issue",
      project,
      issue,
    },
  );
});

test("presents project work with its canonical subject and project filter", () => {
  const [item] = buildInboxItems({
    feed: {
      feed: {
        mentions: [feedItem()],
        needsAction: [],
        activity: [],
        agentActivity: [],
      },
      meta: { since: 0, total: 1, generatedAt: 1_700_000_000 },
    },
  });

  assert.equal(item.subject, "Add project work items to Inbox");
  assert.deepEqual(getInboxTypeLabel(item), {
    text: "Pull request",
    channelLabel: null,
  });
  assert.equal(matchesInboxFilter(item, "project"), true);
  assert.equal(
    matchesInboxFilter(
      {
        ...item,
        item: feedItem({ kind: 9, tags: [["h", "channel-id"]] }),
        groupItems: [feedItem({ kind: 9, tags: [["h", "channel-id"]] })],
      },
      "project",
    ),
    false,
  );
});

test("groups uppercase NIP-34 pull request updates with their root", () => {
  const update = feedItem({
    id: "f".repeat(64),
    kind: 1619,
    createdAt: 1_700_000_100,
    content: "Pushed another commit",
    tags: [
      ["a", REPO_ADDRESS],
      ["E", PR_ID],
      ["p", REVIEWER],
    ],
  });
  const items = buildInboxItems({
    feed: {
      feed: {
        mentions: [feedItem(), update],
        needsAction: [],
        activity: [],
        agentActivity: [],
      },
      meta: { since: 0, total: 2, generatedAt: 1_700_000_100 },
    },
  });

  assert.equal(items.length, 1);
  assert.equal(items[0].id, update.id);
  assert.equal(items[0].subject, "Add project work items to Inbox");
});

test("does not group project events from different repositories", () => {
  const otherRepoAddress = `30617:${"e".repeat(64)}:other`;
  const items = buildInboxItems({
    feed: {
      feed: {
        mentions: [
          feedItem(),
          feedItem({
            id: "f".repeat(64),
            kind: 1619,
            tags: [
              ["a", otherRepoAddress],
              ["E", PR_ID],
              ["p", REVIEWER],
            ],
          }),
        ],
        needsAction: [],
        activity: [],
        agentActivity: [],
      },
      meta: { since: 0, total: 2, generatedAt: 1_700_000_100 },
    },
  });

  assert.equal(items.length, 2);
  assert.notEqual(items[0].conversationId, items[1].conversationId);
});
