import assert from "node:assert/strict";
import test from "node:test";

import { FEED_COALESCE_WINDOW_MS, coalesceFeedItems } from "./coalesce.ts";

function feedItem(overrides = {}) {
  return {
    category: "mention",
    channelId: "channel-1",
    channelName: "ship-room",
    content: "hello",
    createdAt: 1,
    id: "item-1",
    kind: 9,
    pubkey: "agent-a",
    tags: [],
    ...overrides,
  };
}

test("returns null for zero items so nothing is delivered", () => {
  assert.equal(coalesceFeedItems([]), null);
});

test("returns null for a single item so the existing format is preserved", () => {
  assert.equal(coalesceFeedItems([feedItem()]), null);
});

test("folds a burst into one summary counting distinct agents", () => {
  const items = [
    feedItem({ id: "a", pubkey: "agent-a" }),
    feedItem({ id: "b", pubkey: "agent-b" }),
    feedItem({ id: "c", pubkey: "agent-c" }),
    feedItem({ id: "d", pubkey: "agent-a" }),
  ];

  const summary = coalesceFeedItems(items);

  assert.equal(summary.count, 4);
  assert.equal(summary.agentCount, 3);
  assert.equal(summary.title, "3 agents: 4 events");
});

test("surfaces the relay's needs_action partition in the body", () => {
  const items = [
    feedItem({ category: "mention", id: "a", pubkey: "agent-a" }),
    feedItem({ category: "needs_action", id: "b", pubkey: "agent-b" }),
    feedItem({ category: "needs_action", id: "c", pubkey: "agent-c" }),
  ];

  const summary = coalesceFeedItems(items);

  assert.equal(summary.needsActionCount, 2);
  assert.equal(summary.body, "2 items need you");
});

test("uses singular copy for exactly one needs-action item", () => {
  const summary = coalesceFeedItems([
    feedItem({ category: "mention", id: "a", pubkey: "agent-a" }),
    feedItem({ category: "needs_action", id: "b", pubkey: "agent-b" }),
  ]);

  assert.equal(summary.body, "1 item needs you");
});

test("falls back to an update count when nothing needs action", () => {
  const summary = coalesceFeedItems([
    feedItem({ id: "a", pubkey: "agent-a" }),
    feedItem({ id: "b", pubkey: "agent-a" }),
  ]);

  assert.equal(summary.needsActionCount, 0);
  assert.equal(summary.agentCount, 1);
  assert.equal(summary.title, "1 agent: 2 events");
  assert.equal(summary.body, "2 new updates in Buzz");
});

test("summary carries no item content, channel name, or pubkey", () => {
  const summary = coalesceFeedItems([
    feedItem({
      content: "secret token abc123",
      id: "a",
      pubkey: "npub-sensitive-a",
    }),
    feedItem({
      channelName: "private-room",
      content: "another secret",
      id: "b",
      pubkey: "npub-sensitive-b",
    }),
  ]);

  const rendered = `${summary.title} ${summary.body}`;
  for (const leak of [
    "secret",
    "abc123",
    "npub-sensitive-a",
    "npub-sensitive-b",
    "private-room",
    "ship-room",
  ]) {
    assert.equal(
      rendered.includes(leak),
      false,
      `coalesced summary leaked untrusted text: ${leak}`,
    );
  }
});

test("window constant is a positive finite duration", () => {
  assert.equal(Number.isFinite(FEED_COALESCE_WINDOW_MS), true);
  assert.equal(FEED_COALESCE_WINDOW_MS > 0, true);
});
