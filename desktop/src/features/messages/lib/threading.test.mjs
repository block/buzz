import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReplyTags,
  getThreadReference,
  normalizeEventId,
  pTagRoleFor,
  P_TAG_ADDRESSING_MARKER,
  P_TAG_MENTION_MARKER,
} from "./threading.ts";

test("a malformed e-tag value still groups, but never reaches a filter", () => {
  // Two separate concerns, deliberately split. `getThreadReference` is the general
  // thread-grouping primitive, and a value that cannot identify a relay event is
  // still a usable grouping key — so it passes through here. What must not happen
  // is that value reaching an `ids` REQ filter: the relay stores such an event
  // (its NIP-10 resolver ignores the tag rather than rejecting the event, so any
  // member can publish one) and answers the resulting filter with a bare NOTICE —
  // no CLOSED, no EOSE — which this client never resolves, so the request hangs
  // the history timeout and then rejects. `normalizeEventId` is the gate for that,
  // and every filter-building caller must use it.
  for (const bad of ["not-a-real-id", "abc", "z".repeat(64), "a".repeat(63)]) {
    const ref = getThreadReference([
      ["e", "b".repeat(64), "", "root"],
      ["e", bad, "", "reply"],
    ]);
    assert.equal(ref.parentId, bad, `expected ${bad} to survive grouping`);
    assert.equal(
      normalizeEventId(ref.parentId),
      null,
      `expected ${bad} to be gated`,
    );
  }
});

test("an uppercase e-tag value is normalized to lowercase", () => {
  // Hex decodes case-insensitively, so the relay resolves an uppercase id fine —
  // but every comparison is against `event.id`, which is always lowercase. The
  // mismatch read as "parent absent", which relabels a reply a real mention:
  // a second notification on top of the live path's, and a pierced mute.
  const upper = "A".repeat(64);
  const ref = getThreadReference([["e", upper, "", "reply"]]);

  assert.equal(ref.parentId, "a".repeat(64));
  assert.equal(ref.rootId, "a".repeat(64));
});

test("a valid lowercase reference passes through unchanged", () => {
  const root = "1".repeat(64);
  const parent = "2".repeat(64);
  const ref = getThreadReference([
    ["e", root, "", "root"],
    ["e", parent, "", "reply"],
  ]);

  assert.deepEqual(ref, { parentId: parent, rootId: root });
});

test("normalizeEventId accepts only canonical 64-char hex", () => {
  assert.equal(normalizeEventId("f".repeat(64)), "f".repeat(64));
  assert.equal(normalizeEventId("F".repeat(64)), "f".repeat(64));
  assert.equal(normalizeEventId(null), null);
  assert.equal(normalizeEventId(undefined), null);
  assert.equal(normalizeEventId("g".repeat(64)), null);
});

test("a p-tag role is only what the sender actually marked", () => {
  const ME = "a".repeat(64);
  const OTHER = "b".repeat(64);
  const e = (tags) => tags;

  // No marker at all — a sender that predates the markers. This must stay
  // "unknown" and fall through to the parent lookup. Reading it as a mention
  // would put every reply back through the mute it exists to respect.
  assert.equal(pTagRoleFor(e([["p", ME]]), ME), "unknown");
  assert.equal(pTagRoleFor(e([["p", ME, "", "reply"]]), ME), "addressing");
  assert.equal(pTagRoleFor(e([["p", ME, "", "mention"]]), ME), "mention");
  assert.equal(pTagRoleFor(e([["p", OTHER, "", "mention"]]), ME), "none");
  assert.equal(pTagRoleFor(e([]), ME), "none");

  // Mention wins: the sender emits only this marker when we are both the
  // author being answered and someone typed in the body.
  assert.equal(
    pTagRoleFor(
      e([
        ["p", ME, "", "reply"],
        ["p", ME, "", "mention"],
      ]),
      ME,
    ),
    "mention",
  );

  // One bare tag among marked ones still means "ask the parent" for us.
  assert.equal(
    pTagRoleFor(
      e([
        ["p", OTHER, "", "mention"],
        ["p", ME],
      ]),
      ME,
    ),
    "unknown",
  );

  // Case is normalized on both sides.
  assert.equal(
    pTagRoleFor(e([["p", ME.toUpperCase(), "", "reply"]]), ME),
    "addressing",
  );
});

const SELF = "a".repeat(64);
const COUNTERPART = "b".repeat(64);
const TYPED = "c".repeat(64);
const PARENT = "d".repeat(64);

const pTags = (tags) =>
  tags.filter(([name, pubkey]) => name === "p" && pubkey !== SELF);

test("a channel recipient is tagged bare, never as a mention", () => {
  // A DM tags its other participants whether or not anyone typed their names.
  // Claiming `mention` would let a DM thread reply pierce a mute and outrank a
  // real `@you`; bare means "ask the parent", which is the honest answer and the
  // one these tags already got before markers existed.
  const tags = buildReplyTags(
    "chan",
    SELF,
    "parent-id",
    "parent-id",
    [],
    PARENT,
    [COUNTERPART],
  );
  assert.deepEqual(pTags(tags), [
    ["p", COUNTERPART],
    ["p", PARENT, "", P_TAG_ADDRESSING_MARKER],
  ]);
  assert.equal(pTagRoleFor(tags, COUNTERPART), "unknown");
  assert.equal(pTagRoleFor(tags, PARENT), "addressing");
});

test("a channel recipient who wrote the parent gets one addressing tag", () => {
  // The ordinary DM reply: the counterpart is both the other participant and the
  // author being answered.
  const tags = buildReplyTags(
    "chan",
    SELF,
    "parent-id",
    "parent-id",
    [],
    COUNTERPART,
    [COUNTERPART],
  );
  assert.deepEqual(pTags(tags), [
    ["p", COUNTERPART, "", P_TAG_ADDRESSING_MARKER],
  ]);
});

test("a channel recipient who was also typed is a mention", () => {
  const tags = buildReplyTags(
    "chan",
    SELF,
    "parent-id",
    "parent-id",
    [COUNTERPART],
    null,
    [COUNTERPART],
  );
  assert.deepEqual(pTags(tags), [["p", COUNTERPART, "", P_TAG_MENTION_MARKER]]);
  assert.equal(pTagRoleFor(tags, COUNTERPART), "mention");
});

test("reply p-tags come out in the order the backend emits them", () => {
  // The optimistic row must be tag-identical to the event the relay stores:
  // typed mentions, then bare channel recipients, then the addressing tag.
  const tags = buildReplyTags(
    "chan",
    SELF,
    "parent-id",
    "parent-id",
    [TYPED],
    PARENT,
    [COUNTERPART],
  );
  assert.deepEqual(pTags(tags), [
    ["p", TYPED, "", P_TAG_MENTION_MARKER],
    ["p", COUNTERPART],
    ["p", PARENT, "", P_TAG_ADDRESSING_MARKER],
  ]);
});
