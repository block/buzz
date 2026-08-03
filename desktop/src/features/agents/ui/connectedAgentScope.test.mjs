import assert from "node:assert/strict";
import test from "node:test";

import {
  connectedAgentsForCommunity,
  normalizeCommunityUrl,
} from "./connectedAgentScope.ts";

const PRIMARY = "wss://community.example";
const SECONDARY = "wss://other.example";

function agent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    name: "Scout",
    host: "workstation",
    harness: "claude",
    community: PRIMARY,
    createdAt: "2026-07-29T00:00:00Z",
    updatedAt: "2026-07-29T00:00:00Z",
    ...overrides,
  };
}

test("an agent appears only in its recorded community", () => {
  assert.equal(connectedAgentsForCommunity([agent()], PRIMARY).length, 1);
  assert.deepEqual(connectedAgentsForCommunity([agent()], SECONDARY), []);
});

test("legacy records remain visible until reconnected", () => {
  const legacy = agent({ community: undefined });
  assert.equal(connectedAgentsForCommunity([legacy], PRIMARY).length, 1);
  assert.equal(connectedAgentsForCommunity([legacy], SECONDARY).length, 1);
});

test("comparison ignores trailing slashes and case", () => {
  assert.equal(
    connectedAgentsForCommunity(
      [agent({ community: "WSS://COMMUNITY.EXAMPLE/" })],
      PRIMARY,
    ).length,
    1,
  );
  assert.equal(
    normalizeCommunityUrl("  wss://Relay.Example.com// "),
    "wss://relay.example.com",
  );
});

test("an unknown active community does not hide records", () => {
  assert.equal(connectedAgentsForCommunity([agent()], null).length, 1);
  assert.equal(connectedAgentsForCommunity([agent()], "").length, 1);
});

test("mixed communities are separated while legacy records stay visible", () => {
  const agents = [
    agent({ pubkey: "a".repeat(64), community: PRIMARY }),
    agent({ pubkey: "b".repeat(64), community: SECONDARY }),
    agent({ pubkey: "c".repeat(64), community: undefined }),
  ];
  assert.deepEqual(
    connectedAgentsForCommunity(agents, PRIMARY).map((item) => item.pubkey),
    ["a".repeat(64), "c".repeat(64)],
  );
});
