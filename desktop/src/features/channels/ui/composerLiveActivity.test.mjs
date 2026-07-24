import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveLastLiveAt,
  resolveSelectedActivityAgent,
} from "./composerLiveActivity.ts";

const alice = { pubkey: "ALICE-pubkey", name: "Alice" };
const bob = { pubkey: "bob-pubkey", name: "Bob" };

test("explicit selection wins, case-insensitively", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: "bob-pubkey",
    selectedPubkey: "alice-PUBKEY",
    workingAgents: [alice, bob],
  });
  assert.equal(agent, alice);
});

test("falls back to the open session agent", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: "bob-pubkey",
    selectedPubkey: null,
    workingAgents: [alice, bob],
  });
  assert.equal(agent, bob);
});

test("falls back to the first working agent", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: "gone-pubkey",
    selectedPubkey: "also-gone",
    workingAgents: [alice, bob],
  });
  assert.equal(agent, alice);
});

test("selection that stopped working falls through", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: null,
    selectedPubkey: "bob-pubkey",
    workingAgents: [alice],
  });
  assert.equal(agent, alice);
});

test("returns null with no working agents", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: null,
    selectedPubkey: null,
    workingAgents: [],
  });
  assert.equal(agent, null);
});

const CHANNEL = "channel-1";
const OTHER_CHANNEL = "channel-2";

test("deriveLastLiveAt prefers the newest channel-scoped transcript item", () => {
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [],
    archivedEvents: [],
    channelId: CHANNEL,
    transcript: [
      { channelId: CHANNEL, timestamp: "2026-07-23T00:00:01.000Z" },
      { channelId: OTHER_CHANNEL, timestamp: "2026-07-23T00:00:09.000Z" },
      { channelId: CHANNEL, timestamp: "2026-07-23T00:00:05.000Z" },
    ],
  });
  assert.equal(lastLiveAt, Date.parse("2026-07-23T00:00:05.000Z"));
});

test("deriveLastLiveAt sees archived content the panel renders", () => {
  // Regression: archived rows are visible in the preview even when the live
  // transcript window is empty — the pill must not say "No activity yet".
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [],
    archivedEvents: [{ timestamp: "2026-07-20T10:00:00.000Z" }],
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(lastLiveAt, Date.parse("2026-07-20T10:00:00.000Z"));
});

test("deriveLastLiveAt takes the newest across live, archive, and turn anchor", () => {
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [
      { anchorAt: Date.parse("2026-07-23T00:00:30.000Z"), channelId: CHANNEL },
    ],
    archivedEvents: [{ timestamp: "2026-07-23T00:00:10.000Z" }],
    channelId: CHANNEL,
    transcript: [{ channelId: CHANNEL, timestamp: "2026-07-23T00:00:20.000Z" }],
  });
  assert.equal(lastLiveAt, Date.parse("2026-07-23T00:00:30.000Z"));
});

test("deriveLastLiveAt falls back to the active-turn anchor with no items", () => {
  const anchorAt = Date.parse("2026-07-23T00:01:00.000Z");
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [{ anchorAt, channelId: CHANNEL }],
    archivedEvents: [],
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(lastLiveAt, anchorAt);
});

test("deriveLastLiveAt ignores other-channel turns and returns null when idle", () => {
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [{ anchorAt: 1, channelId: OTHER_CHANNEL }],
    archivedEvents: [],
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(lastLiveAt, null);
});
