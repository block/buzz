import assert from "node:assert/strict";
import { test } from "node:test";
import { buildPulseBriefing, briefingPriority } from "./pulseBriefing.ts";

const now = 1_800_000_000;
const me = "me";
const reads = {
  getChannelReadAt: () => null,
  getThreadReadAt: () => null,
  getMessageReadAt: () => null,
  isFollowingThread: () => false,
  isThreadMuted: () => false,
};
const message = (extra = {}) => ({
  id: "message",
  pubkey: "alice",
  author: "Alice",
  body: "Hello",
  createdAt: now - 60,
  ...extra,
});
const conversation = (id, messages = [message()], extra = {}) => ({
  id,
  rootId: id,
  channel: { id: "dm", channelType: "dm" },
  messages,
  ...extra,
});
const build = (items, overrides = {}) =>
  buildPulseBriefing(items, me, { ...reads, ...overrides }, now);

test("DM summary counts conversations once and excludes old, outgoing, pending, and read messages", () => {
  const items = [conversation("one"), conversation("two")];
  assert.equal(build(items)[0].label, "1 DM conversation has unread messages");
  assert.equal(build(items)[0].ids.size, 2);
  for (const marker of [
    "getChannelReadAt",
    "getThreadReadAt",
    "getMessageReadAt",
  ]) {
    assert.equal(build(items, { [marker]: () => now }).length, 0);
  }
  for (const extra of [
    { pubkey: me },
    { pending: true },
    { createdAt: now - 49 * 3600 },
  ]) {
    assert.equal(build([conversation("excluded", [message(extra)])]).length, 0);
  }
  assert.equal(build(items, { isThreadMuted: () => true }).length, 0);
});

test("agent input requests require current, personally relevant agent evidence", () => {
  const request = message({
    isAgent: true,
    pubkey: "scout",
    author: "Scout",
    body: "I'm blocked on access. I need your approval to continue.",
  });
  const item = conversation("request", [request], {
    channel: { id: "engineering", channelType: "stream" },
  });
  assert.equal(
    build([item]).length,
    0,
    "another person's agent is not my action item",
  );
  const personal = { ...item, messages: [{ ...request, ownerPubkey: me }] };
  assert.equal(build([personal])[0].kind, "agent");
  for (const body of [
    "I'm not blocked on anything.",
    "> I'm blocked on access.",
    "```\nI'm blocked on access.\n```",
    "The user said I'm blocked on access.",
  ]) {
    assert.equal(
      build([{ ...personal, messages: [{ ...personal.messages[0], body }] }])
        .length,
      0,
    );
  }
  for (const latest of [
    message({ pubkey: me }),
    { ...request, body: "Resolved. All checks passed.", createdAt: now },
  ]) {
    assert.equal(
      build([{ ...personal, messages: [...personal.messages, latest] }]).length,
      0,
      "a later response retires the request",
    );
  }
});

test("unread mentions and followed threads rank above routine activity", () => {
  const mention = conversation("mention", [message({ tags: [["p", me]] })], {
    channel: { id: "general", channelType: "stream" },
  });
  const groups = build([mention], { isFollowingThread: () => true });
  assert.deepEqual(
    groups.map((group) => group.kind),
    ["mention", "thread"],
  );
  assert.ok(
    briefingPriority("mention", groups) > briefingPriority("routine", groups),
  );
  assert.equal(build([mention], { getMessageReadAt: () => now }).length, 0);
  assert.deepEqual(buildPulseBriefing([mention], undefined, reads, now), []);
});
