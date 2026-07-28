import assert from "node:assert/strict";
import test from "node:test";

import {
  isHighPriorityEventForUser,
  notifyDecisionForEvent,
} from "./shouldNotify.ts";

const PUBKEY = "a".repeat(64);
const OTHER_PUBKEY = "b".repeat(64);
const ROOT_ID = `root-${"0".repeat(59)}`;
const PARENT_ID = `parent-${"0".repeat(57)}`;

const EMPTY = new Set();

/** Returns a minimal RelayEvent with the given tags. */
function makeEvent(tags = [], overrides = {}) {
  return {
    id: `event-${"0".repeat(59)}`,
    pubkey: OTHER_PUBKEY,
    created_at: 1700000000,
    kind: 9,
    tags,
    content: "hello",
    sig: "s".repeat(128),
    ...overrides,
  };
}

const rootTag = (id) => ["e", id, "", "root"];
const replyTag = (id) => ["e", id, "", "reply"];
const pTag = (pubkey) => ["p", pubkey];
const broadcastTag = () => ["broadcast", "1"];

const opts = (overrides = {}) => ({
  participatedRootIds: EMPTY,
  followedRootIds: EMPTY,
  authoredRootIds: EMPTY,
  ...overrides,
});

const unreadFor = (event, pubkey, options) =>
  notifyDecisionForEvent(event, pubkey, options).unread;

test("top-level message (no e-tags) notifies", () => {
  assert.equal(unreadFor(makeEvent([]), PUBKEY, opts()), true);
});

test("top-level message with unrelated p-tag notifies", () => {
  assert.equal(
    unreadFor(makeEvent([pTag(OTHER_PUBKEY)]), PUBKEY, opts()),
    true,
  );
});

test("broadcast reply to unrelated thread notifies", () => {
  const event = makeEvent([replyTag(ROOT_ID), broadcastTag()]);
  assert.equal(unreadFor(event, PUBKEY, opts()), true);
});

test("broadcast reply with root+reply tags notifies", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
  assert.equal(unreadFor(event, PUBKEY, opts()), true);
});

test("thread reply with p-tag mention of currentPubkey notifies", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(unreadFor(event, PUBKEY, opts()), true);
});

test("p-tag mention matching is case-insensitive", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY.toUpperCase())]);
  assert.equal(unreadFor(event, PUBKEY, opts()), true);
});

test("p-tag mention of a different pubkey does not trigger mention path", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(OTHER_PUBKEY),
  ]);
  assert.equal(unreadFor(event, PUBKEY, opts()), false);
});

test("thread reply to participated thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ participatedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("shallow thread reply (root===parent) to participated thread notifies", () => {
  const event = makeEvent([replyTag(ROOT_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ participatedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("thread reply to followed thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ followedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("thread reply to authored thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ authoredRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("thread reply to unrelated thread does not notify", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(unreadFor(event, PUBKEY, opts()), false);
});

test("muted thread reply suppresses participated", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(
      event,
      PUBKEY,
      opts({
        participatedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("muted thread reply suppresses followed", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(
      event,
      PUBKEY,
      opts({
        followedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("muted thread reply suppresses authored", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(
      event,
      PUBKEY,
      opts({
        authoredRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("muted thread reply still notifies when currentPubkey is mentioned via p-tag", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ mutedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("muted rootId does not suppress a top-level (non-reply) message", () => {
  const event = makeEvent([]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ mutedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("omitting mutedRootIds parameter defaults to empty set and still notifies participated", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ participatedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("omitting mutedRootIds for unrelated thread returns false without throwing", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(unreadFor(event, PUBKEY, opts()), false);
});

test("muted shallow thread reply (rootId falls back to parentId) is suppressed", () => {
  const event = makeEvent([replyTag(ROOT_ID)]);
  assert.equal(
    unreadFor(
      event,
      PUBKEY,
      opts({
        participatedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("broadcast reply on a muted thread still notifies (broadcast overrides mute)", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
  assert.equal(
    unreadFor(event, PUBKEY, opts({ mutedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("empty currentPubkey skips p-tag check — muted thread is suppressed", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    unreadFor(
      event,
      "",
      opts({
        participatedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("empty currentPubkey with participated thread still notifies (no mute)", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    unreadFor(event, "", opts({ participatedRootIds: new Set([ROOT_ID]) })),
    true,
  );
});

test("isHighPriorityEventForUser returns true when p-tag matches currentPubkey", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), true);
});

test("isHighPriorityEventForUser returns true for broadcast reply", () => {
  const event = makeEvent([replyTag(ROOT_ID), broadcastTag()]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), true);
});

test("isHighPriorityEventForUser returns false when no matching p-tag and no broadcast tag", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(OTHER_PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), false);
});

test("isHighPriorityEventForUser p-tag matching is case-insensitive", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY.toUpperCase())]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), true);
});

test("isHighPriorityEventForUser returns false when currentPubkey is empty", () => {
  // Short-circuits before p-tag check; broadcast absent so also false
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, ""), false);
});

test("isHighPriorityEventForUser returns false for event with no tags at all", () => {
  const event = makeEvent([]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), false);
});

// ── decision tiers (channel-blind defaults) ───────────────────────────────────

test("top-level post decision alerts without mention tier", () => {
  assert.deepEqual(notifyDecisionForEvent(makeEvent([]), PUBKEY, opts()), {
    unread: true,
    alert: true,
    highPriority: false,
  });
});

test("p-tag mention decision is the full mention tier", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.deepEqual(notifyDecisionForEvent(event, PUBKEY, opts()), {
    unread: true,
    alert: true,
    highPriority: true,
  });
});

test("broadcast reply decision is mention tier at level 'all'", () => {
  const event = makeEvent([replyTag(ROOT_ID), broadcastTag()]);
  assert.deepEqual(notifyDecisionForEvent(event, PUBKEY, opts()), {
    unread: true,
    alert: true,
    highPriority: true,
  });
});

test("followed thread reply decision alerts without mention tier", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.deepEqual(
    notifyDecisionForEvent(
      event,
      PUBKEY,
      opts({ followedRootIds: new Set([ROOT_ID]) }),
    ),
    { unread: true, alert: true, highPriority: false },
  );
});

test("ignored thread reply decision is all-false", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.deepEqual(notifyDecisionForEvent(event, PUBKEY, opts()), {
    unread: false,
    alert: false,
    highPriority: false,
  });
});
