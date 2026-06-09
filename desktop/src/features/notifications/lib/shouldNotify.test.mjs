import assert from "node:assert/strict";
import test from "node:test";

import {
  isHighPriorityEventForUser,
  shouldNotifyForEvent,
} from "./shouldNotify.ts";

// ── Fixtures ─────────────────────────────────────────────────────────────────

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

// ── 1. Top-level message → always true ──────────────────────────────────────

test("top-level message (no e-tags) notifies", () => {
  assert.equal(shouldNotifyForEvent(makeEvent([]), PUBKEY, opts()), true);
});

test("top-level message with unrelated p-tag notifies", () => {
  assert.equal(
    shouldNotifyForEvent(makeEvent([pTag(OTHER_PUBKEY)]), PUBKEY, opts()),
    true,
  );
});

// ── 2. Broadcast reply → always true ─────────────────────────────────────────

test("broadcast reply to unrelated thread notifies", () => {
  const event = makeEvent([replyTag(ROOT_ID), broadcastTag()]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("broadcast reply with root+reply tags notifies", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

// ── 3. Thread reply with p-tag mention → always true ─────────────────────────

test("thread reply with p-tag mention of currentPubkey notifies", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("p-tag mention matching is case-insensitive", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY.toUpperCase())]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("p-tag mention of a different pubkey does not trigger mention path", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(OTHER_PUBKEY),
  ]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), false);
});

// ── 4. Thread reply to participated thread → true ────────────────────────────

test("thread reply to participated thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("shallow thread reply (root===parent) to participated thread notifies", () => {
  const event = makeEvent([replyTag(ROOT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── 5. Thread reply to followed thread → true ────────────────────────────────

test("thread reply to followed thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ followedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── 6. Thread reply to authored thread → true ────────────────────────────────

test("thread reply to authored thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ authoredRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── 7. Thread reply to unrelated thread → false ───────────────────────────────

test("thread reply to unrelated thread does not notify", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), false);
});

// ── 8. Muted thread suppresses participated ───────────────────────────────────

test("muted thread reply suppresses participated", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
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

// ── 9. Muted thread suppresses followed ──────────────────────────────────────

test("muted thread reply suppresses followed", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
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

// ── 10. Muted thread suppresses authored ─────────────────────────────────────

test("muted thread reply suppresses authored", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
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

// ── 11. Muted thread with p-tag mention still notifies ───────────────────────

test("muted thread reply still notifies when currentPubkey is mentioned via p-tag", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── 12. Muted top-level message still notifies ───────────────────────────────

test("muted rootId does not suppress a top-level (non-reply) message", () => {
  const event = makeEvent([]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── 13. Default parameter — mutedRootIds omitted ─────────────────────────────

test("omitting mutedRootIds parameter defaults to empty set and still notifies participated", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("omitting mutedRootIds for unrelated thread returns false without throwing", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), false);
});

// ── 14. Muted shallow thread reply (single replyTag, no rootTag) ────────────

test("muted shallow thread reply (rootId falls back to parentId) is suppressed", () => {
  const event = makeEvent([replyTag(ROOT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
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

// ── 15. Muted thread + broadcast reply still notifies ───────────────────────

test("broadcast reply on a muted thread still notifies (broadcast overrides mute)", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── 16. Empty currentPubkey with p-tag present ──────────────────────────────

test("empty currentPubkey skips p-tag check — muted thread is suppressed", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
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
    shouldNotifyForEvent(
      event,
      "",
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

// ── isHighPriorityEventForUser ────────────────────────────────────────────────

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
