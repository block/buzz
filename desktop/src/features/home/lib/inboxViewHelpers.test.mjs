import assert from "node:assert/strict";
import test from "node:test";

import { formatTimelineMessages } from "../../messages/lib/formatTimelineMessages.ts";
import { getConfigNudgeAuthorPubkey } from "../../messages/ui/configNudgeAuthPubkey.ts";
import {
  filterActivityInboxItems,
  getContextMessageDepth,
  getReactionTargetId,
  hasInboxThreadContext,
  isInboxThreadContextEvent,
  matchesActivityAllView,
  matchesActivityCustomView,
  matchesInboxFilter,
  toInboxContextMessage,
  toTimelineMessage,
} from "./inboxViewHelpers.ts";

test("Activity uses the dedicated reminder list instead of feed reminder rows", () => {
  const message = { item: { kind: 9 } };
  const reminder = { item: { kind: 40007 } };
  const items = [message, reminder];

  assert.equal(filterActivityInboxItems(items, false), items);
  assert.deepEqual(filterActivityInboxItems(items, true), [message]);
});

test("hasInboxThreadContext finds replies in the grouped row or loaded context", () => {
  const root = { tags: [["h", "channel"]] };
  const reply = {
    tags: [
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ],
  };

  assert.equal(
    hasInboxThreadContext({ item: root, groupItems: [root, reply] }),
    true,
  );
  assert.equal(
    hasInboxThreadContext({ item: root, groupItems: [root] }, [reply]),
    true,
  );
});

test("hasInboxThreadContext keeps standalone and broadcast activity unthreaded", () => {
  const root = { tags: [["h", "channel"]] };
  const broadcastReply = {
    tags: [
      ["h", "channel"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ],
  };

  assert.equal(
    hasInboxThreadContext({ item: root, groupItems: [root] }),
    false,
  );
  assert.equal(
    hasInboxThreadContext({
      item: broadcastReply,
      groupItems: [broadcastReply],
    }),
    false,
  );
});

// --- matchesInboxFilter ---

test("matchesInboxFilter returns true for the 'all' filter regardless of categories", () => {
  assert.equal(matchesInboxFilter({ categories: [] }, "all"), true);
  assert.equal(matchesInboxFilter({ categories: ["mentions"] }, "all"), true);
});

test("Activity All excludes generic top-level channel traffic", () => {
  const owned = new Set(["owned-agent"]);
  assert.equal(
    matchesActivityAllView(
      {
        categories: ["activity"],
        item: {
          channelType: "stream",
          pubkey: "human",
          tags: [["h", "channel"]],
        },
      },
      owned,
    ),
    false,
  );
});

test("Activity All includes each personally relevant message source", () => {
  const owned = new Set(["owned-agent"]);
  const cases = [
    {
      categories: ["activity"],
      item: { channelType: "dm", pubkey: "human", tags: [] },
    },
    {
      categories: ["mention"],
      item: { channelType: "stream", pubkey: "human", tags: [] },
    },
    {
      categories: ["needs_action"],
      item: { channelType: "stream", pubkey: "human", tags: [] },
    },
    {
      categories: ["activity"],
      item: {
        channelType: "stream",
        pubkey: "human",
        tags: [["e", "root", "", "reply"]],
      },
    },
    {
      categories: ["activity"],
      item: { channelType: "stream", pubkey: "OWNED-AGENT", tags: [] },
    },
  ];

  for (const item of cases) {
    assert.equal(matchesActivityAllView(item, owned), true);
  }
});

test("Activity All excludes generic updates from agents the user does not own", () => {
  assert.equal(
    matchesActivityAllView(
      {
        categories: ["agent_activity"],
        item: {
          channelType: "stream",
          pubkey: "somebody-elses-agent",
          tags: [],
        },
      },
      new Set(["owned-agent"]),
    ),
    false,
  );
});

test("matchesInboxFilter matches when the category is present", () => {
  assert.equal(
    matchesInboxFilter({ categories: ["mentions", "activity"] }, "mentions"),
    true,
  );
});

test("matchesInboxFilter is false when the category is absent", () => {
  assert.equal(
    matchesInboxFilter({ categories: ["activity"] }, "mentions"),
    false,
  );
  assert.equal(matchesInboxFilter({ categories: [] }, "mentions"), false);
});

test("owned-agent filtering uses the representative event author", () => {
  const owned = new Set(["owned-agent"]);
  assert.equal(
    matchesInboxFilter(
      {
        categories: ["activity"],
        item: { pubkey: "OWNED-AGENT" },
      },
      "agent_activity",
      owned,
    ),
    true,
  );
  assert.equal(
    matchesInboxFilter(
      {
        categories: ["agent_activity"],
        item: { pubkey: "somebody-elses-agent" },
      },
      "agent_activity",
      owned,
    ),
    false,
  );
});

test("matchesInboxFilter matches thread rows by thread tags", () => {
  const replyItem = {
    id: "reply",
    kind: 9,
    pubkey: "author",
    content: "reply",
    createdAt: 2,
    channelId: "channel",
    channelName: "bugs",
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "parent", "", "reply"],
    ],
    category: "activity",
  };
  const rootItem = {
    id: "root",
    kind: 9,
    pubkey: "author",
    content: "root",
    createdAt: 1,
    channelId: "channel",
    channelName: "bugs",
    tags: [["h", "channel"]],
    category: "activity",
  };

  assert.equal(
    matchesInboxFilter(
      {
        categories: ["activity"],
        item: replyItem,
      },
      "thread",
    ),
    true,
  );

  assert.equal(
    matchesInboxFilter(
      {
        categories: ["mention", "activity"],
        item: { ...replyItem, category: "mention" },
      },
      "thread",
    ),
    true,
  );

  assert.equal(
    matchesInboxFilter(
      {
        categories: ["mention", "activity"],
        groupItems: [rootItem, replyItem],
        item: { ...rootItem, category: "mention" },
      },
      "thread",
    ),
    true,
  );

  assert.equal(
    matchesInboxFilter(
      {
        categories: ["activity"],
        item: rootItem,
      },
      "thread",
    ),
    false,
  );
});

test("matchesActivityCustomView uses union matching across selected sources", () => {
  const item = {
    categories: ["mention"],
    item: {
      id: "dm",
      pubkey: "person",
      channelType: "dm",
      tags: [],
    },
  };
  const custom = {
    dms: false,
    mentions: true,
    threads: false,
    needsAction: false,
    agentReplies: false,
    dueReminders: false,
    drafts: false,
  };

  assert.equal(matchesActivityCustomView(item, custom, new Set()), true);
  assert.equal(
    matchesActivityCustomView(
      { ...item, categories: [] },
      { ...custom, dms: true },
      new Set(),
    ),
    true,
  );
});

test("matchesActivityCustomView only includes replies from owned agents", () => {
  const item = {
    categories: ["activity"],
    item: { id: "reply", pubkey: "OWNED", channelType: "channel", tags: [] },
  };
  const custom = {
    dms: false,
    mentions: false,
    threads: false,
    needsAction: false,
    agentReplies: true,
    dueReminders: false,
    drafts: false,
  };

  assert.equal(
    matchesActivityCustomView(item, custom, new Set(["owned"])),
    true,
  );
  assert.equal(
    matchesActivityCustomView(item, custom, new Set(["other"])),
    false,
  );
});

// --- getReactionTargetId ---

test("getReactionTargetId returns the last e-tag target id", () => {
  const tags = [
    ["e", "first"],
    ["p", "somebody"],
    ["e", "second"],
  ];
  assert.equal(getReactionTargetId(tags), "second");
});

test("getReactionTargetId returns null when there is no e-tag", () => {
  assert.equal(getReactionTargetId([["p", "somebody"]]), null);
  assert.equal(getReactionTargetId([]), null);
});

test("getReactionTargetId ignores e-tags with a missing/non-string id", () => {
  // Trailing malformed e-tag should be skipped in favor of the valid one.
  const tags = [["e", "valid"], ["e"]];
  assert.equal(getReactionTargetId(tags), "valid");
});

// --- getContextMessageDepth ---

function event(id, parentId) {
  // A "reply" e-tag is how getThreadReference resolves a parent.
  const tags = parentId ? [["e", parentId, "", "reply"]] : [];
  return {
    id,
    pubkey: "x",
    created_at: 0,
    kind: 9,
    tags,
    content: "",
    sig: "",
  };
}

test("getContextMessageDepth is 0 for a root message", () => {
  const root = event("root", null);
  const map = new Map([[root.id, root]]);
  assert.equal(getContextMessageDepth(root, map), 0);
});

test("getContextMessageDepth counts ancestors present in the map", () => {
  const root = event("root", null);
  const mid = event("mid", "root");
  const leaf = event("leaf", "mid");
  const map = new Map([
    [root.id, root],
    [mid.id, mid],
    [leaf.id, leaf],
  ]);
  assert.equal(getContextMessageDepth(leaf, map), 2);
  assert.equal(getContextMessageDepth(mid, map), 1);
});

test("getContextMessageDepth stops when a parent is missing from the map", () => {
  // leaf -> mid (present) -> absent root. Depth counts only the present hop.
  const mid = event("mid", "absent-root");
  const leaf = event("leaf", "mid");
  const map = new Map([
    [mid.id, mid],
    [leaf.id, leaf],
  ]);
  assert.equal(getContextMessageDepth(leaf, map), 1);
});

test("getContextMessageDepth does not loop forever on a cycle", () => {
  // a -> b -> a. The `seen` set must terminate the walk.
  const a = event("a", "b");
  const b = event("b", "a");
  const map = new Map([
    [a.id, a],
    [b.id, b],
  ]);
  // From a: hop to b (depth 1); b's parent is a, already seen -> stop.
  assert.equal(getContextMessageDepth(a, map), 1);
});

// --- isInboxThreadContextEvent ---

function channelEvent(id, tags = []) {
  return {
    id,
    pubkey: "x",
    created_at: 0,
    kind: 9,
    tags: [["h", "channel-a"], ...tags],
    content: "",
    sig: "",
  };
}

test("isInboxThreadContextEvent rejects stale events from a different thread", () => {
  const selection = {
    selectedChannelId: "channel-a",
    selectedEventId: "selected-reply",
    selectedParentId: "selected-parent",
    selectedThreadRootId: "selected-root",
  };

  assert.equal(
    isInboxThreadContextEvent(
      channelEvent("old-root", [["e", "old-root", "", "root"]]),
      selection,
    ),
    false,
  );
  assert.equal(
    isInboxThreadContextEvent(
      channelEvent("old-reply", [
        ["e", "old-root", "", "root"],
        ["e", "old-parent", "", "reply"],
      ]),
      selection,
    ),
    false,
  );
});

test("isInboxThreadContextEvent keeps selected thread root, parent, selected event, and descendants", () => {
  const selection = {
    selectedChannelId: "channel-a",
    selectedEventId: "selected-reply",
    selectedParentId: "selected-parent",
    selectedThreadRootId: "selected-root",
  };

  assert.equal(
    isInboxThreadContextEvent(channelEvent("selected-root"), selection),
    true,
  );
  assert.equal(
    isInboxThreadContextEvent(channelEvent("selected-parent"), selection),
    true,
  );
  assert.equal(
    isInboxThreadContextEvent(channelEvent("selected-reply"), selection),
    true,
  );
  assert.equal(
    isInboxThreadContextEvent(
      channelEvent("descendant", [
        ["e", "selected-root", "", "root"],
        ["e", "selected-reply", "", "reply"],
      ]),
      selection,
    ),
    true,
  );
});

// --- config-nudge pipeline: toInboxContextMessage → toTimelineMessage ---
//
// The inbox detail pane renders messages through two mappings that a real
// event's `kind` and `signerPubkey` must survive: HomeView's
// toInboxContextMessage and InboxMessageRow's toTimelineMessage. An earlier
// version of the HomeView mapping dropped both fields, structurally
// disabling the config-nudge card on the inbox surface (the raw
// ```buzz:config-nudge fence rendered instead). These tests run a real event
// through formatTimelineMessages and both mappings, then assert against the
// same gate helper InboxMessageRow calls.

const NUDGE_CHANNEL_ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const NUDGE_AGENT_SIGNER =
  "2222222222222222222222222222222222222222222222222222222222222222";
const NUDGE_HUMAN_SIGNER =
  "1111111111111111111111111111111111111111111111111111111111111111";

function makeNudgeEvent(overrides = {}) {
  return {
    id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    pubkey: NUDGE_AGENT_SIGNER,
    kind: 9,
    created_at: 1_700_000_000,
    content: "**Fizz** needs configuration.\n\n```buzz:config-nudge\n{}\n```",
    tags: [["h", NUDGE_CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function mapThroughInboxPipeline(event) {
  const [formatted] = formatTimelineMessages([event], null, undefined, null);
  const contextMessage = toInboxContextMessage(formatted, {
    eventById: new Map([[event.id, event]]),
    fallbackAuthorPubkey: event.pubkey,
    profiles: undefined,
    selectedItemId: event.id,
  });
  return toTimelineMessage(contextMessage);
}

test("inbox mappings preserve kind and signerPubkey so the nudge gate can pass", () => {
  const message = mapThroughInboxPipeline(makeNudgeEvent());

  assert.equal(message.kind, 9);
  assert.equal(message.signerPubkey, NUDGE_AGENT_SIGNER);
  assert.equal(message.createdAt, 1_700_000_000);
  assert.equal(
    getConfigNudgeAuthorPubkey(
      message,
      (pubkey) => pubkey === NUDGE_AGENT_SIGNER,
    ),
    NUDGE_AGENT_SIGNER,
  );
});

test("inbox mappings ignore a spoofed actor tag", () => {
  const message = mapThroughInboxPipeline(
    makeNudgeEvent({
      pubkey: NUDGE_HUMAN_SIGNER,
      tags: [
        ["h", NUDGE_CHANNEL_ID],
        ["actor", NUDGE_AGENT_SIGNER],
      ],
    }),
  );

  // An untrusted actor tag cannot replace the display author. The signer also
  // stays human, so the config-nudge gate rejects the message.
  assert.equal(message.pubkey?.toLowerCase(), NUDGE_HUMAN_SIGNER);
  assert.equal(message.signerPubkey, NUDGE_HUMAN_SIGNER);
  assert.equal(
    getConfigNudgeAuthorPubkey(
      message,
      (pubkey) => pubkey === NUDGE_AGENT_SIGNER,
    ),
    undefined,
  );
});

test("inbox mappings: unknown signer never enables the card", () => {
  const message = mapThroughInboxPipeline(makeNudgeEvent());

  assert.equal(
    getConfigNudgeAuthorPubkey(message, () => false),
    undefined,
  );
});
