import assert from "node:assert/strict";
import test from "node:test";

import { availableRelayAgents } from "./availableRelayAgents.ts";

const ME = "a".repeat(64);
const OTHER = "b".repeat(64);

function agent(name, pubkey, overrides = {}) {
  return {
    pubkey,
    ownerPubkey: null,
    name,
    agentType: "relay",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "offline",
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("lists only relay agents the current identity may instruct", () => {
  const result = availableRelayAgents(
    [
      agent("Pokebalzer", "1".repeat(64), {
        respondTo: "allowlist",
        respondToAllowlist: [ME],
        status: "online",
      }),
      agent("Relay Master", "2".repeat(64), { ownerPubkey: ME }),
      agent("Someone else's", "3".repeat(64), { ownerPubkey: OTHER }),
      agent("Shared channel", "4".repeat(64), {
        respondTo: "anyone",
        channelIds: ["general"],
      }),
      agent("Unshared", "5".repeat(64), {
        respondTo: "anyone",
        channelIds: ["private-elsewhere"],
      }),
    ],
    new Set(["general"]),
    ME,
  );

  assert.deepEqual(
    result.map(({ name }) => name),
    ["Pokebalzer", "Relay Master", "Shared channel"],
  );
});

test("deduplicates pubkeys and keeps status-first ordering", () => {
  const duplicate = "6".repeat(64);
  const result = availableRelayAgents(
    [
      agent("Offline", "7".repeat(64), {
        respondTo: "allowlist",
        respondToAllowlist: [ME],
      }),
      agent("Online", duplicate.toUpperCase(), {
        respondTo: "allowlist",
        respondToAllowlist: [ME],
        status: "online",
      }),
      agent("Duplicate", duplicate, {
        respondTo: "allowlist",
        respondToAllowlist: [ME],
      }),
    ],
    new Set(),
    ME,
  );

  assert.deepEqual(
    result.map(({ name }) => name),
    ["Online", "Offline"],
  );
});
