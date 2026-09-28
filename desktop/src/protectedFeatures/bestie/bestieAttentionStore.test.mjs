import assert from "node:assert/strict";
import test from "node:test";

import {
  __resetBestieAttentionStoreForTests,
  clearBestieUnreadMessage,
  getBestieHasUnreadMessage,
  getBestieSeenAgentCreatedAt,
  ingestBestieAgentMessageCreatedAt,
  isBestieSurfaceOpen,
  markBestieAgentMessagesSeen,
  noteBestieAgentMessageCreatedAt,
  setBestiePopoverOpen,
  setBestieViewingDm,
} from "./bestieAttentionStore.ts";

test("agent message while closed sets unread; open clears", () => {
  __resetBestieAttentionStoreForTests();
  assert.equal(getBestieHasUnreadMessage(), false);
  ingestBestieAgentMessageCreatedAt(100);
  assert.equal(getBestieHasUnreadMessage(), true);
  setBestiePopoverOpen(true);
  assert.equal(isBestieSurfaceOpen(), true);
  assert.equal(getBestieHasUnreadMessage(), false);
  setBestiePopoverOpen(false);
  ingestBestieAgentMessageCreatedAt(100);
  assert.equal(getBestieHasUnreadMessage(), false);
  ingestBestieAgentMessageCreatedAt(200);
  assert.equal(getBestieHasUnreadMessage(), true);
  setBestieViewingDm(true);
  assert.equal(getBestieHasUnreadMessage(), false);
  clearBestieUnreadMessage();
  markBestieAgentMessagesSeen(300);
  ingestBestieAgentMessageCreatedAt(250);
  assert.equal(getBestieHasUnreadMessage(), false);
});

test("ring stays off while popover open even if unread flag races", () => {
  __resetBestieAttentionStoreForTests();
  setBestiePopoverOpen(true);
  // Simulate a stale ingest that flipped the internal flag before open synced.
  ingestBestieAgentMessageCreatedAt(100);
  assert.equal(getBestieHasUnreadMessage(), false);
  assert.equal(getBestieSeenAgentCreatedAt(), 100);
  setBestiePopoverOpen(false);
  assert.equal(getBestieHasUnreadMessage(), false);
});

test("noteBestieAgentMessageCreatedAt advances seen without pulsing", () => {
  __resetBestieAttentionStoreForTests();
  noteBestieAgentMessageCreatedAt(50);
  assert.equal(getBestieSeenAgentCreatedAt(), 50);
  assert.equal(getBestieHasUnreadMessage(), false);
  noteBestieAgentMessageCreatedAt(80);
  ingestBestieAgentMessageCreatedAt(80);
  assert.equal(getBestieHasUnreadMessage(), false);
  ingestBestieAgentMessageCreatedAt(90);
  assert.equal(getBestieHasUnreadMessage(), true);
  noteBestieAgentMessageCreatedAt(120);
  assert.equal(getBestieHasUnreadMessage(), true);
  assert.equal(getBestieSeenAgentCreatedAt(), 120);
  ingestBestieAgentMessageCreatedAt(120);
  assert.equal(getBestieHasUnreadMessage(), true);
});

test("markBestieAgentMessagesSeen clears stale attention", () => {
  __resetBestieAttentionStoreForTests();
  ingestBestieAgentMessageCreatedAt(10);
  assert.equal(getBestieHasUnreadMessage(), true);
  markBestieAgentMessagesSeen(10);
  assert.equal(getBestieHasUnreadMessage(), false);
});
