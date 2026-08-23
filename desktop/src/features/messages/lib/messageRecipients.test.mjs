import assert from "node:assert/strict";
import test from "node:test";

import { messageRecipients } from "./messageRecipients.ts";
import { MENTION_TAG_CAP } from "./threading.ts";

function channel(overrides = {}) {
  return {
    id: "dm-1",
    name: "DM",
    channelType: "dm",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: ["OWNER", "AGENT"],
    participantPubkeys: ["owner", "agent"],
    participants: [],
    lastMessageAt: null,
    archivedAt: null,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

const stream = (overrides = {}) =>
  channel({
    channelType: "stream",
    memberPubkeys: ["owner", "agent"],
    ...overrides,
  });

test("plain DM messages p-tag every recipient except the sender", () => {
  assert.deepEqual(messageRecipients(channel(), "owner"), {
    mentions: [],
    addressed: ["agent"],
  });
});

test("a DM counterpart is addressed by the channel, not mentioned", () => {
  // The distinction is the point: as a mention this tag would pierce a mute and
  // outrank a real `@you` in the mention feed. Nobody typed the counterpart's
  // name — the channel has exactly one other participant.
  const { mentions, addressed } = messageRecipients(channel(), "owner");
  assert.deepEqual(mentions, []);
  assert.deepEqual(addressed, ["agent"]);
});

test("a DM participant who was also typed counts only as a mention", () => {
  assert.deepEqual(messageRecipients(channel(), "OWNER", ["AGENT", "third"]), {
    mentions: ["agent", "third"],
    addressed: [],
  });
});

test("stream messages preserve explicit-mention semantics", () => {
  assert.deepEqual(messageRecipients(stream(), "owner", []), {
    mentions: [],
    addressed: [],
  });
});

test("stream replies leave the addressing tag to the backend", () => {
  // The parent's author is no longer folded in here. `resolve_thread_ref`
  // already fetches the parent event on the way to the thread root, so the
  // backend emits that tag itself — marked, and without depending on a client
  // cache that misses for every channel not opened this session.
  assert.deepEqual(messageRecipients(stream(), "owner", [], "AGENT"), {
    mentions: [],
    addressed: [],
  });
});

test("replying to your own message reserves no addressing slot", () => {
  // Only observable at the cap. Away from it both branches return the same
  // empty result, so the self check has nothing to prove: the backend never
  // emits an addressing tag for the signer, so no slot is held back for one.
  const typed = Array.from({ length: 60 }, (_, i) => `mention-${i}`);
  const { mentions, addressed } = messageRecipients(
    stream(),
    "owner",
    typed,
    "owner",
  );
  assert.equal(mentions.length, MENTION_TAG_CAP);
  assert.deepEqual(addressed, []);
  assert.ok(!mentions.includes("owner"));
});

test("a reply to someone already mentioned reserves no second slot", () => {
  // The parent author is already typed, so they are emitted once as a mention
  // and cost one slot rather than two — again visible only where the cap bites.
  const typed = Array.from({ length: 60 }, (_, i) => `mention-${i}`);
  const { mentions } = messageRecipients(stream(), "owner", typed, "mention-0");
  assert.equal(mentions.length, MENTION_TAG_CAP);
  assert.equal(mentions.filter((pubkey) => pubkey === "mention-0").length, 1);
});

test("mentions keep their slots ahead of channel recipients at the cap", () => {
  // 50 typed mentions fill the cap outright, so a DM counterpart who was never
  // typed is what gives — the same order the single merged list produced.
  const typed = Array.from({ length: 60 }, (_, i) => `mention-${i}`);
  const { mentions, addressed } = messageRecipients(channel(), "owner", typed);
  assert.equal(mentions.length, 50);
  assert.deepEqual(addressed, []);
});

test("the addressing slot is reserved out of the mention list", () => {
  const typed = Array.from({ length: 60 }, (_, i) => `mention-${i}`);
  const { mentions } = messageRecipients(stream(), "owner", typed, "parent");
  assert.equal(mentions.length, 49);
});

test("a reserved slot is not taken twice when the parent is a DM recipient", () => {
  // `agent` is both the DM's other participant and the parent's author. The
  // backend emits one tag for them, marked as addressing, so no extra slot is
  // needed and the mention list keeps its full cap.
  const typed = Array.from({ length: 60 }, (_, i) => `mention-${i}`);
  const { mentions } = messageRecipients(channel(), "owner", typed, "agent");
  assert.equal(mentions.length, 50);
});
