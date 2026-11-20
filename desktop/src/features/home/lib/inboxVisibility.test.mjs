import assert from "node:assert/strict";
import test from "node:test";

import { buildInboxItems } from "./inbox.ts";
import { isInboxItemVisible } from "./inboxViewHelpers.ts";

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const DM_CHANNEL_ID = "8ad375a7-6990-4b22-985f-e3fd34f634d7";

const channels = [
  { id: CHANNEL_ID, name: "buzz-bugs", channelType: "stream" },
  { id: DM_CHANNEL_ID, name: "dm-alice", channelType: "dm" },
];

function feedWith({ mentions = [], activity = [] }) {
  return {
    feed: { mentions, needsAction: [], activity, agentActivity: [] },
    meta: { since: 0, total: 0, generatedAt: 0 },
  };
}

function item({ id, createdAt, category, tags, channelId = CHANNEL_ID }) {
  return {
    id,
    kind: 9,
    pubkey: "author",
    content: "hello",
    createdAt,
    channelId,
    channelName: "",
    tags,
    category,
  };
}

function reply(id, createdAt, category) {
  return item({
    id,
    createdAt,
    category,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "root-event", "", "root"],
      ["e", "root-event", "", "reply"],
    ],
  });
}

// User B mentions me in a thread reply; later replies from others do not.
function threadWithMention({ mentionReadAt }) {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      mentions: [reply("mention-reply", 1, "mention")],
      activity: [
        reply("later-1", 2, "activity"),
        reply("later-2", 3, "activity"),
      ],
    }),
    getMessageReadAt: (messageId) =>
      messageId === "mention-reply" ? mentionReadAt : null,
    getThreadReadAt: () => null,
  });
  return inboxItem;
}

function visible(inboxItem, overrides = {}) {
  return isInboxItemVisible(inboxItem, {
    doneSet: new Set(),
    filter: "mention",
    localUnreadSet: new Set(),
    selectedConversationId: null,
    unreadOnly: true,
    ...overrides,
  });
}

test("Unread + Mentions hides a thread whose only mention was already read", () => {
  const inboxItem = threadWithMention({ mentionReadAt: 1 });

  assert.equal(visible(inboxItem), false);
});

test("Mentions without Unread still lists a thread whose mention was read", () => {
  const inboxItem = threadWithMention({ mentionReadAt: 1 });

  assert.equal(visible(inboxItem, { unreadOnly: false }), true);
});

test("Unread + Mentions lists a thread whose mention is still unread", () => {
  const inboxItem = threadWithMention({ mentionReadAt: null });

  assert.equal(visible(inboxItem), true);
});

test("Unread + Activity keeps a thread whose mention was read but later replies are unread", () => {
  const inboxItem = threadWithMention({ mentionReadAt: 1 });

  assert.equal(visible(inboxItem, { filter: "all" }), true);
});

test("Unread + Mentions keeps a read-mention thread the user marked unread", () => {
  const inboxItem = threadWithMention({ mentionReadAt: 1 });

  assert.equal(
    visible(inboxItem, { localUnreadSet: new Set([inboxItem.id]) }),
    true,
  );
});

test("Unread + Mentions keeps the selected conversation visible", () => {
  const inboxItem = threadWithMention({ mentionReadAt: 1 });

  assert.equal(
    visible(inboxItem, { selectedConversationId: inboxItem.conversationId }),
    true,
  );
});

test("Unread + Mentions keeps a thread whose root post mentions me", () => {
  // The root's read state is not tracked per message here, so it must not be
  // treated as read just because the replies under it are evaluated.
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      mentions: [
        item({
          id: "root-event",
          createdAt: 1,
          category: "mention",
          tags: [["h", CHANNEL_ID]],
        }),
      ],
      activity: [reply("later-1", 2, "activity")],
    }),
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
  });

  assert.equal(visible(inboxItem), true);
});

test("Unread + Mentions hides a DM whose mention is older than the read marker", () => {
  const dmMessage = (id, createdAt, category) =>
    item({
      id,
      createdAt,
      category,
      channelId: DM_CHANNEL_ID,
      tags: [["h", DM_CHANNEL_ID]],
    });
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      mentions: [dmMessage("dm-mention", 1, "mention")],
      activity: [dmMessage("dm-later", 3, "activity")],
    }),
    getChannelReadAt: () => 2,
  });

  assert.equal(visible(inboxItem), false);
  assert.equal(visible(inboxItem, { filter: "all" }), true);
});

test("Unread hides a row that is fully read", () => {
  const inboxItem = threadWithMention({ mentionReadAt: 1 });

  assert.equal(
    visible(inboxItem, { doneSet: new Set([inboxItem.id]), filter: "all" }),
    false,
  );
});
